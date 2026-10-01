# Terminology servers

openEHR records carry coded values: a diagnosis, a route of administration, a
laboratory unit. Some codes come from openEHR's own terminology; others must be
checked against an external code system such as SNOMED CT or LOINC. FerroEHR
serves the bundled openEHR terminology in-process, and can additionally validate
coded values against any number of external FHIR terminology servers at once.

<!-- toc -->

## The bundled openEHR terminology

The server ships the openEHR terminology bundle (Terminology 3.1.0) and uses it
by default, with no configuration and no external dependency. It answers the
questions the platform needs while validating and querying: which terminologies
exist, whether a code belongs to one, what a term's rubric is, whether one code
subsumes another, and whether a code is a member of a value set.

Enumeration is always the bundle's job. A lookup or a membership test goes to the
bundle when the bundle knows that terminology, and only otherwise to a
configured external server.

### Exposing the lookups over REST

You can also expose these lookups over a small read-only REST surface. It is a
FerroEHR extension (no openEHR REST contract defines a terminology API) and it
is **off by default**; while disabled, every route answers `404` as if it were
not mounted. Turn it on with `FERROEHR__TERMINOLOGY__API_ENABLED=true` and it
serves:

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/terminology` | list terminologies |
| `GET` | `/terminology/{terminology_id}` | describe one terminology |
| `GET` | `/terminology/{terminology_id}/term/{code}` | look up a term |
| `GET` | `/terminology/{terminology_id}/subsumes?ref_code=&candidate=` | subsumption test |
| `GET` | `/terminology/{terminology_id}/value_set/{value_set_id}` | get a value set |
| `GET` | `/terminology/{terminology_id}/value_set/{value_set_id}/validate?candidate_code=&at_date=` | test membership |

All paths are relative to the API base path, `/ferroehr/rest/openehr/v1`. An
unknown terminology or value set is a `404`; a missing required query parameter
is a `400`.

## External FHIR terminology servers

> [!NOTE]
> **The CDR is only ever a client of the terminology server.** FerroEHR does not
> implement one: you run a FHIR terminology server and point the CDR at it by
> URL. The one that ships with the product is
> [FerroTERM](#ferroterm-beside-the-cdr), started by the quickstart's
> terminology overlay; any other server speaking the FHIR terminology
> operations (Ontoserver, Snowstorm, HAPI FHIR) works the same way.

External terminology is off by default, and while it is off nothing is
requested: validation uses the in-process bundle alone. The keys live under
`[terminology.external]`; the full table, including timeouts, caching, OAuth2
and mutual TLS, is on
[Integrations](../installation/config-integrations.md#terminology). The minimum
is a master switch and one provider:

```toml
[terminology.external]
enabled = true

[terminology.external.providers.default]
type = "fhir"
url = "https://tx.example.org/fhir"
```

Enabling the section with no provider configured is a **boot error**, not a
silent fall back to the bundle. So is an empty provider URL, an
`oauth2_client` naming no configured client, and half a mutual-TLS identity:
a control you configured either works or the server refuses to start.

### What it changes at commit time

The mechanism is openEHR's own **archetype constraint binding**. Where a
template binds an archetype constraint code (an `ac` code) to a query against an
external terminology, the specification puts the resolver *outside* the CDR: the
archetype holds an identifier for a query, and the query itself is defined in the
terminology server.

With external terminology enabled, committing a composition resolves those
bindings for every bound coded value the instance actually carries, against the
server the binding's terminology routes to:

- The code **is** in the value set → the commit proceeds.
- The code is **not** in the value set → `422`, naming the path, the code, and
  the bound query. That is a real constraint violation, so `fail_on_error` does
  not change it.
- The value set **could not be resolved** → `fail_on_error` decides, see
  [below](#when-the-terminology-server-cannot-answer).

Only bound external queries leave the process. openEHR and the bundle's own
local terminologies are still answered in-process.

> [!WARNING]
> The composition's `terminology_id` travels verbatim as the FHIR `system`
> parameter, and no openEHR specification defines a mapping between
> `terminology_id` values (`SNOMED-CT`) and FHIR system URIs
> (`http://snomed.info/sct`). If your archetypes and your terminology server
> disagree on that spelling, align them in the terminology-server configuration.
> The CDR does not rewrite the value.

### What it changes when an ADL 2 archetype is uploaded

An ADL 2 archetype's `term_bindings` are checked too (AOM2 validity rule
VETDF). A binding target is a URI in the IHTSDO model, `…/id/<code>`, and the
CDR takes it apart before asking: `http://snomed.info/id/50121007` (or the
`snomedct.info` host the ADL 2 specification's own examples use) is asked as
`system=http://snomed.info/sct`, `code=50121007`, per the SNOMED CT URI Standard;
`http://loinc.org/id/LA6742-6` as `system=http://loinc.org`; any other `…/id/…`
URI as the URI before `/id/`. The outer binding key (`SNOMED-CT`, `Snomed`,
`LOINC`, however the author spelled it) only picks the provider. The server's
answer then decides:

- the code **exists** → accepted;
- the code system is served and the code is **not in it** → `422`, naming the
  rule, the binding and the code;
- the code system is **not served** by the terminology server (or the server
  cannot say) → accepted with a warning in the log. AOM2 puts it this way:
  codes for inaccessible terminologies "should be flagged with a warning
  indicating that no verification was possible", so a CDR wired at a server
  that carries only LOINC still stores archetypes with SNOMED CT bindings.

The same rule holds with no external terminology server configured at all:
nothing is asked and nothing is refused.

### Which FHIR operation is used

Membership is tested with `ValueSet/$validate-code` by default: one direct
yes/no with the least payload. A server that does not offer it can be switched to
`$expand` plus a membership test with `operation = "expand"` on that provider.
This is a per-provider configuration choice, not an automatic fallback: the
server does not retry a failed `$validate-code` as an `$expand`, so set the
operation your server actually supports.

Responses are cached per provider (decoded, not raw), so a burst of commits
validating the same codes does not become one HTTPS round trip per code. A
response that is not a valid FHIR resource is treated as an upstream fault rather
than partially read, and takes the same path as an unreachable server.

### Several terminology servers at once

Real deployments bind to more than one terminology: SNOMED CT from one server,
LOINC or a national code system from another. **Every** entry under
`[terminology.external.providers]` is materialised at startup, and
`[terminology.external.routes]` decides which one answers each call:

```toml
[terminology.external]
enabled = true

[terminology.external.providers.snomed]
type = "fhir"
url = "https://snomed.example.org/fhir"

[terminology.external.providers.loinc]
type = "fhir"
url = "https://loinc.example.org/fhir"

# Terminology id or system URI -> provider name.
[terminology.external.routes]
"SNOMED-CT" = "snomed"
"http://snomed.info/sct" = "snomed"
"http://loinc.org" = "loinc"
```

Selection is deliberately mechanical, so you can predict which server answers:

1. The caller offers candidate keys in priority order: a terminology id, a
   system URI, a value-set URL, the AQL service flavour.
2. The first candidate with a route entry wins. Keys are matched
   **case-insensitively and whole-string**, never as a prefix.
3. Otherwise the provider named `default` answers, or, when exactly one
   provider is configured, that one.
4. With two or more providers and no `default`, an unrouted terminology has no
   server at all, and the call falls back to local behaviour.

Step 4 is a useful way to make routing mistakes loud instead of silent: add a
`default` only when you genuinely want a catch-all server. A route naming a
provider that does not exist fails at **startup**, never at request time.

Routing applies everywhere terminology is consulted: the `/terminology/*`
extension API, AQL `TERMINOLOGY(…)` resolution, and the commit-time binding
checks above.

### Authenticating to a server

A provider that needs a bearer token references a client-credentials client by
name; the token is cached and renewed shortly before it expires. The client
secret should come from a mounted file rather than an inline value;
`client_secret_file` under
`[terminology.external.oauth2_clients.<name>]`. A provider that authenticates
with a certificate instead gets its mutual-TLS identity **per provider**, because
the certificate is issued by that server's PKI. Both are configured on
[Integrations](../installation/config-integrations.md#authenticating-to-a-terminology-server).

There is no option to disable certificate verification. Server-certificate and
hostname verification are always on; a private-PKI trust bundle changes *which*
anchors are trusted, never *whether* the server is verified.

### When the terminology server cannot answer

`fail_on_error` decides what happens when a bound value set cannot be resolved at
all, whether the server is unreachable, answers an error, or does not know the
value set:

- `false` (the default, *fail-open*): the composition is accepted and a warning
  is logged. The availability of an external service does not block clinical
  writes.
- `true` (*fail-closed*): the composition is rejected with a validation error
  naming the unresolved binding.

A code that *is* resolved and turns out not to be a member is a different
matter: that is a constraint violation and is rejected under either setting.

Pick deliberately. Fail-closed means commits stop while your terminology server
is down; fail-open means they are accepted unvalidated. There is no third option
that gives you both.

## FerroTERM beside the CDR

[FerroTERM](https://github.com/FerroHEALTH/FerroTERM) is the terminology server
FerroEHR ships with: the `docker-compose.terminology.yml` overlay of the
quickstart starts it beside the CDR and points `[terminology.external]` at
`http://ferroterm:8080/r4b`, the [compose page](../installation/compose.md#the-terminology-overlay-ferroterm)
has the commands. The hosted sandbox at <https://sandbox.ferroehr.eu> runs the
same pair on one machine, so the coded-text binding you see resolving there is a
real round trip to a real terminology server.

What the sandbox shows, and how it is arranged:

- **The server is not on the public surface.** FerroTERM answers on the
  sandbox's private compose network alone; there is no route to it at
  `sandbox.ferroehr.eu`, its own browser UI is off, and the CDR is its only
  caller. You exercise it through the CDR: commit a composition against the
  `cnf.tpl.dv_coded_text_binding_sct` template with a code inside its bound
  value set and it is accepted, with a code outside it and the answer is `422`;
  the `/terminology/*` routes look codes up; AQL `TERMINOLOGY()` expands a value
  set into a `MATCHES` operand.
- **Fail-open**, the shipped default: a binding the server cannot resolve is
  accepted. The sandbox declares this posture and the register entry that
  records why neither posture is spec-mandated (AMB-172).
- **The content is the shaped seed**: two code systems and two value sets under
  the reserved `example.test` domain, with no licensed terminology in them.

### SNOMED CT on a public sandbox

Serving SNOMED CT to the public is a licensed activity, and the arrangement
above is what the SNOMED CT Affiliate Licence Agreement (April 2023,
[snomed.org/get-snomed](https://www.snomed.org/get-snomed)) asks for. Clause 2.2.4
permits systems "made available to the general public for accessing and/or
retrieving any part of the International Release and/or data encoded using the
foregoing", provided users "are not able to extract any substantial portion of
SNOMED CT" and no fee is charged for access. Clause 2.7 requires "reasonable
measures to ensure that the International Release (and any part of it) cannot be
accessed or downloaded from the Licensee's systems except by authorised users".
A raw FHIR terminology endpoint open to anonymous callers would let anyone walk
a code system or expand large value sets; per-code operations through the CDR,
behind its rate limit, do not.

The sandbox serves the **SNOMED CT International Edition, release 20260901**
(`http://snomed.info/sct/900000000000207008/version/20260901`), loaded by the
operator under their Affiliate Licence from an index built off the machine. The
International Edition rather than a national one: it needs the Affiliate Licence
alone (a Member's national release also needs an agreement with that Member), its
English displays fit an international audience, and it is the lighter of the two.
Every surface showing that content carries the notice the licence prescribes:

> This material includes SNOMED Clinical Terms® (SNOMED CT®) which is used by
> permission of the International Health Terminology Standards Development
> Organisation (IHTSDO). All rights reserved. SNOMED CT®, was originally created
> by The College of American Pathologists. "SNOMED" and "SNOMED CT" are
> registered trademarks of the IHTSDO.

No SNOMED CT content is in this repository, in any image, or in CI; the index
exists only on the sandbox machine. This is a description of the arrangement,
not legal advice: the licence text is the authority and the Member's conditions
apply in each territory.

### LOINC on the sandbox

The sandbox also serves **LOINC version 2.83** (`http://loinc.org`), from an
index built the same way. The LOINC licence
([loinc.org/license](https://loinc.org/license)) grants use and distribution
"for any commercial or non-commercial purpose" without fees and names "online
terminology services" among the permitted products, on three conditions this
deployment meets: the notice below is available where the service's terms are
stated, every LOINC code is shown with one of its LOINC display names (the FHIR
operations return the long common name), and the version is stated. Where a
LOINC record carries a third-party copyright notice of its own, that notice
travels with the record.

> This material contains content from LOINC (http://loinc.org). LOINC is
> copyright © Regenstrief Institute, Inc. and the Logical Observation
> Identifiers Names and Codes (LOINC) Committee and is available at no cost
> under the license at http://loinc.org/license. LOINC® is a registered United
> States trademark of Regenstrief Institute, Inc.

Neither index is in the repository, an image or CI; both exist only on the
sandbox machine, beside the licence-free shaped seed the compose profile ships.

## Running one locally (development and CI)

The quickest local terminology server is the quickstart's
[terminology overlay](../installation/compose.md#the-terminology-overlay-ferroterm):
FerroTERM beside the CDR, the shaped seed served, the CDR wired, one command
and no checkout. The conformance lane still runs its own server: from a
checkout of the repository, the conformance stack can start a HAPI FHIR JPA
server beside the CDR, seeded with the same synthetic code systems and value
sets over its FHIR API (the lane moves to FerroTERM under #3085):

```bash
docker compose -p ferroehr-cnf --project-directory . --profile terminology \
  -f docker/sut-ferroehr.yml -f docker/sut-terminology.yml up -d --wait ferroehr
```

The profile starts the terminology server (host port `8090` by default,
`FERROEHR_TERMINOLOGY_PORT`) plus a one-shot seeding container that uploads the
fixtures over the server's own FHIR API and verifies `$validate-code` and
`$expand` before exiting, so a misconfigured server fails there rather than
inside a later run. The overlay file is what points the CDR at it, by switching
on the `[terminology.external]` providers the development configuration already
carries in the disabled state.

None of this touches the downloadable quickstart Compose file on its own, which
uses the in-process openEHR terminology only; the terminology overlay is the
opt-in that adds FerroTERM to it.

> [!WARNING]
> The seeded content is synthetic and lives under the reserved `example.test`
> domain: one hierarchical code system shaped like SNOMED CT and one shaped like
> LOINC, each with an enumerated value set. It carries no licensed terminology
> content. Point the providers at a real server, and for SNOMED CT hold the
> appropriate licence, for anything beyond experimentation.

The terminology container mounts no volume, so its seeded content lives inside
the container only: re-create it and the seed is gone. Re-run the profile (or
just the seeding container) afterwards.

> [!TIP]
> Seeding your own server is the same shape: upload the CodeSystem and ValueSet
> resources your templates reference over plain FHIR REST (`PUT` to
> `/fhir/CodeSystem/<id>` and `/fhir/ValueSet/<id>`). A FHIR terminology server
> starts empty, and a value set it does not hold is an unresolved binding, which
> your `fail_on_error` setting then decides the fate of.

## On Kubernetes

Providers and routes are maps, so they are supplied as chart values rather than
environment variables; the `config` passthrough renders them verbatim into the
server's configuration file
([Any server setting is reachable](../installation/kubernetes.md#any-server-setting-is-reachable)):

```yaml
# values.yaml
config:
  terminology:
    api_enabled: true
    external:
      enabled: true
      fail_on_error: true      # fail-closed: an unresolvable binding rejects the commit
      providers:
        default:
          type: fhir
          url: https://tx.example.org/fhir
secrets:
  # only when a provider uses OAuth2; keyed by client name
  terminologyOauth2ClientSecrets: {}
```

**Before you enable it:** a reachable terminology server, a decision on
`fail_on_error`, and (if the chart's default-deny egress policy is on) an
egress rule that admits the server, or every call fails as a timeout.
**To turn it off**, set `config.terminology.external.enabled: false`; validation
falls back to the in-process bundle and no external call is made.

## Terminology in AQL

Query authors can constrain a match to a value set with the AQL `TERMINOLOGY()`
function: `TERMINOLOGY('expand', …)` resolves a value set and merges its codes
into a `matches` list at query-analysis time, so the planner sees an ordinary
value list. A `terminology://…` operand in a `matches` list is expanded the same
way. See [Querying with AQL](../querying-aql.md) for the query surface.

Operations with no defined comparison semantics in AQL are typed rejections
rather than silent wrong answers.
