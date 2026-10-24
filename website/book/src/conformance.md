# Conformance

FerroEHR makes a measured claim: it is an openEHR-spec-conformant Clinical Data
Repository, and that claim is backed by a test run you can reproduce. This
chapter explains what conformance means here, how to run the suite (against
this server, against another CDR, or against any deployed endpoint you point it
at) and how to read the artefacts it produces: the report, the
statement, the certificate, and the cross-server comparison matrix.

<!-- toc -->

## What is measured

Conformance is checked by **[Veredictum](https://github.com/Vernum-Projecten/Veredictum)**,
a separate CNF 2.0 reference runner. It lives in its own repository, has the
same maintainer as FerroEHR, and is consumed here at a pinned version. It is a data-driven
interpreter over a committed, machine-readable catalogue authored from the openEHR
Conformance framework itself: protocol-neutral case cores anchored on the official
platform test schedule (case ids follow the schedule's own naming, e.g.
`I_EHR_SERVICE.create_ehr-main`), per-operation bindings mapping every outcome to
its cited wire expectation, closed vocabularies for outcomes/selectors/captures, a
provenance-stamped corpus (the official openEHR Robot data sets re-adjudicated to
spec-text-only evidence), and a typed ambiguity register; a specification silence
is never resolved privately. Every expectation traces to specification text, never
to any server's observed behaviour. The catalogue spans the schedule's chapters:

| Chapter | Scope |
|---|---|
| EHR / EHR_STATUS | EHR service and status operations |
| COMPOSITION / CONTRIBUTION / DIRECTORY | Clinical content, change sets, folder trees |
| DEFINITION | ADL 1.4 + ADL 2 template and stored-query provisioning |
| QUERY | AQL query execution with committed result-set grounds |
| CONTENT | Reference-Model and archetype-constraint accept/reject tables |
| DEMOGRAPHIC / ADMIN / MESSAGING | Party, admin, and messaging services |
| SYSTEM | The `OPTIONS` capability/conformance manifest |
| SF | Simplified formats — FLAT and STRUCTURED commit/read, context, examples |
| SEC / SIG | Authenticated access, authorization separation, audit accountability, and version signing in both depths |
| SMART | SMART App Launch discovery and resource-scope enforcement |
| PERF | The measured performance classes ([Performance](performance.md)) |

The run is what turns cases into a claim. **Verdicts are computed, never
asserted**: a pure function rolls per-case outcomes up through the
capability→tier matrix from the CNF Profiles book into Core / Standard / Options /
SEC-BASIC profile verdicts, honouring the party statement's declared capabilities
and option selections. A case whose wire does not exist on the technology profile,
or whose ground a shared server cannot establish, is recorded as _not applicable
with a machine-readable citation_ rather than silently omitted.

> [!NOTE]
> The runner publishes a JSON Schema for every artefact family it writes, so a
> consumer can validate a record without trusting the tool that produced it. The
> instrument is built from the currently pinned specifications (AQL 1.1.0,
> Terminology 3.1.0, ITS-REST 1.1.0, and the Reference Model generation the
> system under test runs, for FerroEHR the default `development` profile,
> RM 1.2.0; the composed stack takes the profile from configuration, so a
> `stable`-profile run is the same catalogue against the released generation).
> The upstream Robot suites are reference material; their official data fixtures
> enter the corpus only as provenance-stamped re-adjudications.

## The current result

The whole conformance story in one picture: every capability of the claims
matrix, grouped by profile tier, colored AND glyph-marked by the evidence its
cases produced (both charts are generated from the committed runner artefacts and
regenerate-and-diff guarded in CI; no number on them is hand-typed):

![Capability conformance heat grid](conformance-assets/conformance-heat-grid.svg)

The same run broken down two levels deep: a header per schedule chapter with its
total, then one bar per **band**, the surface a case actually exercises (EHR
resource, EHR_STATUS, COMPOSITION, …), with the exact outcome counts printed
beside every row. Every band the taxonomy declares is drawn, so one with no case
for this run shows as an explicit `no cases` row rather than disappearing, and a
hatched segment marks cited-N/A so it reads as neither a pass nor a failure:

![Schedule outcomes by chapter and band](conformance-assets/conformance-chapter-bars.svg)

The published run against FerroEHR reports:

<!-- Generated at build time from docs/conformance/ferroehr/results.json by
     scripts/render/conformance-stats.sh — never hand-type numbers here (CI:
     scripts/checks/conformance-numbers.sh). -->
{{#include ../generated/conformance-stats.md}}

Cases that did not execute are not-applicable with a machine-readable citation (an
unrealized wire on this technology profile, an undeclared option branch, or a
ground a shared server cannot establish), never silent omissions. Options
aggregates optional capabilities under the Profiles book's "any passes" rule.

## Any server can be assessed

The runner is deliberately not tied to FerroEHR — it is a separate project with
its own release line, so the instrument and the system it judges do not share a
build. It assesses **any openEHR CDR reachable over HTTP** and emits the same
artefact set for each system under test, into its own directory:

- **FerroEHR** (the default): the composed stack built from the current sources.
  This is the project's own gate: the committed artefacts are regenerated and
  diff-checked, so a change that moves a verdict cannot land quietly.
- **EHRbase:** `CONF_SUT=ehrbase` composes the official `ehrbase/ehrbase` image
  (with its companion PostgreSQL) on fresh volumes and runs the same catalogue
  with EHRbase's own committed party set. Its measured artefacts feed the
  [comparison page](comparison.md).
- **Bring your own endpoint:** point the runner at any deployed CDR by URL and
  credentials, with its own party set: an *ixit* naming the instances and
  credential environment variables, and a *statement* (the ICS) declaring the
  capabilities and ambiguity-register options the vendor claims. Option branches
  the ICS does not declare are excused as not-applicable with a citation, in the
  ISO/IEC 9646 tradition of test selection. No code or adapter is needed; a
  target is a configuration entry.

The ixit is also where a deployment declares the facts no openEHR operation
exposes, each of which switches on the cases that depend on it:

| Declaration | What it tells the runner |
|---|---|
| `environment` | The hardware, cores, memory, storage and topology a measured run happened on — mandatory for a performance run |
| `containers` | The composed containers, enabling database-side attribution and deterministic maintenance settling |
| `system_id` | The identifier the server stamps into commit audits and the version ids it mints |
| `dump_location` | A path on the server's own file system the admin dump/load operations may use |
| `signing` | The version-signing mode the deployment realizes (digest or openPGP) |
| `smart` | That the deployment runs the SMART resource-server role, and which test issuer it trusts |
| `terminology` | The terminology query servers it is wired to, the namespaces each answers for, and what it does with a bound value set it cannot resolve |

A party that declares none of these has the dependent cases recorded
not-applicable rather than checked against a guess.

That silence has a cost, and the pipeline makes it visible before a run. The
instrument's ixit schema also defines parameters per instance (the
`administrative` posture of a principal, an instance's own `signing`,
`terminology` or `spec_profile` when it differs from the party's), and an
absent one is undeclared, never a default: every case that requires it is
guarded out. `scripts/checks/ixit-declarations.sh` reads that parameter set
from the pinned schema itself and refuses to run the catalogue while an
instance leaves one undeclared, unless `ixit-undeclared.json` beside the ixit
records why that instance leaves it so (an unauthenticated instance has no
principal to be administrative, for example). A pin bump that introduces a
parameter therefore fails the pipeline until the party declares or adjudicates
it, instead of quietly losing the cases it guards.

## Running the suite yourself

The suite runs against a real, deployed server (the same container image and
stack a deployment uses) so the wire under test is always the production
artefact, never a re-wired in-process stub. From a checkout with Docker
available:

```bash
# our server, from the current sources (the default)
bash scripts/conformance.sh

# EHRbase, from the official images
CONF_SUT=ehrbase bash scripts/conformance.sh

# any deployed CDR, by URL (credentials via the SUT_* variables the
# ixit references)
CONF_SUT=byo CONF_BASE_URL=https://your-host/ferroehr/rest/openehr/v1 \
  SUT_USER=user SUT_PASS=password bash scripts/conformance.sh
```

The script brings up the selected stack on fresh volumes (for `byo` it manages
nothing), executes the committed catalogue, computes the verdicts through the pure
pipeline, and writes the artefacts to `docs/conformance/<sut-name>/` before
tearing the stack down.

Useful knobs: a case-id filter as the first argument, `CONF_IXIT` /
`CONF_STATEMENT` for a custom party set, `CONF_OUT` for a different artefact root,
`CONF_NO_COMPOSE` to run against an already-deployed stack, `SKIP_BUILD` to
compose a published image instead of building from source, and the runner's own
`verdicts` subcommand to recompute the documents from a previous `results.json`
without re-running.

### The postures a run covers

Some behaviour exists only in a particular server configuration. Rather than
splitting those into separate runs whose records would have to be merged by hand
(which is exactly how a claim stops being reproducible) the pipeline brings up
**four deployments of the same image** and covers every posture in the **one**
committed record:

- the **primary** deployment runs the SMART resource-server role with fail-closed
  scopes and a trusted test issuer, digest version signing, and FerroTERM as its
  one terminology server, reachable, in the **fail-open** posture;
- a **stable-profile** deployment shares the primary's database and serves the
  released openEHR generation set, with the same terminology server;
- an **openPGP** deployment, in its own compose project on remapped ports, runs
  openPGP version signing and the **fail-closed** terminology posture, wired to a
  terminology server nothing answers on;
- a **fail-open** deployment, also in its own compose project, runs the
  fail-open posture wired to a terminology server nothing answers on.

The reason is the same in every case: a running server realizes exactly one
signing depth, one generation set, and one unresolvable-value-set behaviour, and
it is wired to one terminology server, which is either reachable or not. Testing
each claim means running the deployment that realizes it. The ixit declares each
deployment as its own instance, and the cases that check those properties address
it by name.

Two consequences worth knowing:

- **SMART is the standard posture.** The SMART discovery
  document, the resource-scope grammar, and the fail-closed `403` are executable
  cases in the same record as everything else, driven by principals presenting
  minted Bearer tokens with the roles and resource scopes each case needs. The
  tokens are signed by a committed test issuer: public test key material for the
  harness, never usable for anything else. A system under test whose ixit declares
  no SMART block (EHRbase) records those cases not-applicable with the citation
  instead.
- **External terminology is part of the standard posture too.** An archetype can
  constrain a coded element to a value set only an external terminology query
  server can resolve, so the pipeline composes a real FHIR R4B server beside the
  CDR, seeded with synthetic test code systems and value sets. That covers the
  terminology-routed surface — AQL `TERMINOLOGY()` resolved through the routed
  server, and commit-time validation of a bound value set, accepted for a member
  code and refused for a non-member. What a deployment does when the value set
  cannot be resolved *at all* is not decided by any openEHR text, so it is a
  declared posture rather than a verdict; both branches execute, each on a
  deployment whose terminology server is unreachable. With one server per
  deployment, the case that needs two distinct reachable servers records
  not-applicable by its own guard.

A measured performance or stress run adds one more posture: rate limiting is
turned off for the duration, because the instruments deliberately offer load past
the server's knee and a throttled request would measure the limiter instead of the
server. Both instruments refuse to write a record if the server answered any
`429`, so a measurement can never be silently limiter-shaped.

## Reading the artefacts

A run writes machine records and three human-readable documents to
`docs/conformance/<sut-name>/`. Each has a distinct job.

### The machine records

`results.json` is the party results record: one outcome per case with its
rows-driven coverage, failing step and reason where applicable, and the excusing
citation for every not-applicable entry, alongside the system-under-test identity,
the runner's verification-pack status, the technology profile, and the ixit
digest. `verdicts.json` is the computed verdict report, and
`run-exceptions.json` registers anything the interpreter itself could not cover.
Every other artefact (the three documents, the badges, the charts) is generated
from these; nothing downstream is hand-edited.

### The conformance report

`CONFORMANCE_REPORT.md` is the honest, scoped record of _this run_: the system
under test, the outcome counts, the per-capability evidence rollup, the
machine-computed profile verdicts, and every not-applicable entry with its
excusing citation. Read this when you want to know exactly what happened and why
any case did not run.

### The conformance statement

`CONFORMANCE_STATEMENT.md` is the concise, generated claim: the supported
specification versions, the declared external data formats (JSON and XML), and the
profile results. Every line is a pure function of the machine verdicts, so the
statement can never claim more than the run proves.

### The conformance certificate

`CONFORMANCE_CERTIFICATE.md` follows the structure of the openEHR conformance
certificate template: the system under test, the scope of test, and a
per-capability profile report showing which capabilities are required in each
profile, what each was verified against, and whether each passed. The
**Realization** column separates capabilities verified over released ITS-REST
operations from any verified over routes a product serves of its own design — the
latter never gate an openEHR profile tier. Where the certificate carries a
measured run, its **Workload Coverage** table additionally shows which claimed
capabilities the hospital-simulation load actually exercised; a capability the
simulation does not reach must carry an adjudicated exclusion, printed with its
reason, and the runner's validation gate refuses an artefact tree that leaves such
a row undecided. It is emitted for **every** assessed system — FerroEHR, EHRbase,
or your own — and always identifies itself as a framework assessment with the
claim computed from the attached run; it is never an official openEHR
certification. This is the document to hand to a procurement or evaluation
reviewer who wants the capability-by-capability picture.

### The comparison matrix

The multi-system record is fully generated from the two committed
results/verdicts sets (ours and EHRbase's): profile verdicts, the
capability-by-capability evidence matrix, and failure tables in both directions:
measured numbers only, no editorial adjustment, both directions always published.
It renders as the [comparison page](comparison.md).

> [!TIP]
> The conformance badges in the project README are generated from the same run
> and carry the measured amounts (per-profile capability counts, the overall
> driven-case count, the earned performance class). A badge can never show a pass
> unless the machine verdict does, so a green badge is a claim you can
> immediately reproduce with `scripts/conformance.sh`.

## What conformance does not cover

The catalogue measures the openEHR platform surface, including the simplified
(FLAT/STRUCTURED) formats chapter of the ITS-REST specification. It deliberately
does not stand in for a performance benchmark: durations recorded during the
functional run are telemetry only, and the measured classes own that claim
([Performance](performance.md)).

The other honest boundary is the gap between openEHR's *service* model and its
*released* REST wire. Several service operations were never surfaced as
endpoints (listing an EHR's contributions, counting stored templates or queries,
deleting a template or a stored query) so a case that addresses one has no wire
to drive on this technology profile. Those cases are excused through the
schedule's typed ambiguity register, with the citation printed in the report, and
reported as an explicit scope exclusion on the certificate; never a silent pass
and never an unavoidable failure. FerroEHR does serve routes of its own design
for several of them (see
[Admin & messaging APIs](operations-admin-apis.md) and the archetype routes in
[Templates & validation](templates-validation.md)); those are marked
`extension` on the certificate and never gate an openEHR profile tier.
