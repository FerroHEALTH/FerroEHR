# Claims review

Every public text about FerroEHR is read, once per release, against EHDS
Art. 28 (Regulation (EU) 2025/327). The article reads: "In the information
sheet, instructions for use or other information accompanying EHR systems, and
in the advertising of EHR systems, it shall be prohibited to use text, names,
trademarks, pictures and figurative or other signs that may mislead the
professional user \[…\] with regard to their intended purpose, interoperability
and security by:

- (a) ascribing functions and properties to the EHR system which it does not
  have;
- (b) failing to inform the professional user of likely limitations related to
  interoperability or security features of the EHR system in relation to its
  intended purpose;
- (c) suggesting uses for the EHR system other than those stated to form part
  of the intended purpose in the technical documentation."

This page is the record of those reviews: what was read, what was found, what
was changed. It also carries the re-assessment of EHDS Annex II 2.5. The
measure for point (c) is the [intended-purpose statement](intended-purpose.md).

<!-- toc -->

> [!WARNING]
> The quotations are from the Official Journal text vendored at
> [`docs/law/eu/ehds/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/ehds/text.html)
> ([EUR-Lex](https://eur-lex.europa.eu/eli/reg/2025/327/oj)). Art. 28 takes
> "professional user" from Article 3, point (8), of Regulation (EU) 2018/1807,
> which the repository does not carry, so these pages do not restate that
> definition. A text that passes this review has been read against Art. 28 by
> the manufacturer; no authority has examined it.

## What is reviewed, and how

The texts in scope, each release:

- the repository `README.md` and the landing page at `ferroehr.eu`;
- every page of this book, except the pages generated from the tracker and
  from committed measurement records, whose generators are reviewed instead;
- `SECURITY.md`;
- the READMEs of the published `openehr-*` crates;
- the OCI labels of the three images (`docker/Dockerfile`,
  `docker/viewer/Dockerfile`, `docker/postgres/Dockerfile`) and the Helm
  chart's description;
- the public sandbox at `sandbox.ferroehr.eu`.

Each claim about a function, a property, interoperability or security is
checked against the code, the configuration defaults and the committed
records, and against the intended-purpose statement. A misleading claim is
fixed in the same change as the review. Measured numbers are never typed into
a text: the conformance and performance figures come from generators over the
committed run records, and a CI check fails a page whose numbers disagree
with them.

## The review of 2026-10-06

Read on the tree after commit `d6539d470`, before FerroEHR 4.3.5.

### Changed

| # | Text | Claim | Art. 28 | Finding | Now reads |
|---|---|---|---|---|---|
| 1 | README, opening | "Every compliance claim it makes is machine-verified" | (a) | Only the openEHR conformance numbers are machine-verified. The regulatory pages are reviewed by hand, and "compliance" invites the regulatory reading | "Every conformance claim it makes is machine-verified" |
| 2 | README, "What makes it different" | "Compliance you can verify." | (a) | as row 1 | "Conformance you can verify." |
| 3 | README, licence paragraph and "Security & operations" | "Multi-tenancy \[…\] fully integrated: each tenant is an isolated logical openEHR system, enforced by PostgreSQL row-level security" | (a) | FerroEHR is single-tenant, with no tenant column, row policy or tenancy configuration ([one instance, one organisation](../security.md#one-instance-one-organisation)) | "One instance per organisation: FerroEHR is single-tenant \[…\]"; multi-tenancy removed from the feature list |
| 4 | README, "Security & operations" | national identifiers "sealed under a per-tenant key" | (a), (b) | there are no tenants, and the sealing is off by default | "sealed under a key the deployment holds \[…\] (off by default)" |
| 5 | README, "Security & operations" | "IHE ATNA-compliant system log (DICOM audit messages over TLS syslog)" | (a), (b) | "compliant" claims a property nobody assessed; the syslog sink is off by default and ships with UDP; the default sink is the local hash-chained store | the access log, its two renderings, the local store and the optional forwarding, described as they ship |
| 6 | README, "Security & operations" | "Hardened by default" | (b) | TLS is off by default, and the shipped `deployment_profile` is `sandbox`, which must not hold real patient data | "Hardening built in", with both limitations stated |
| 7 | README, "Integration" | the FHIR R4B connectors, with no limitation | (b) | the in-tree connector was removed in favour of FerroBRIDGE in v4.3.6 (#3080) | the removal is stated |
| 8 | Landing page, title, `og:` and `twitter:` descriptions, structured data | "Spec-compliant and measured", "spec-conformant and measured" | (a) | states conformity as a property; what exists is a measured openEHR conformance record | "openEHR conformance, measured" |
| 9 | Landing page, hero note | "every compliance claim machine-verified" | (a) | as row 1 | "every conformance number machine-verified" |
| 10 | Landing page, conformance section | "Compliance you can verify." and "Every release runs the complete conformance catalogue against the live server" | (a) | the release pipeline does not run the catalogue; the committed record measured FerroEHR 4.3.0 | "Conformance you can verify."; the record "names the FerroEHR version it measured" |
| 11 | Landing page, feature grid heading | "Everything the openEHR standard asks for" | (a) | claims complete coverage of the standard, which nothing measures | "The openEHR platform services" |
| 12 | Landing page, Security card | "multi-tenancy" | (a) | as row 3 | "One instance per organisation." |
| 13 | Book, [Introduction](../introduction.md) | "Compliance you can run yourself" | (a) | as row 1 | "Conformance you can run yourself" |
| 14 | Book, [System architecture](../concepts/architecture.md) | "why the compliance claims are checkable", "the compliance claims are machine-derived" | (a) | as row 1 | "conformance claims" |
| 15 | Book, [Beyond the core](../beyond-core/index.md) | "Security, multi-tenancy, and the audit trail" | (a) | as row 3 | "Security, instance separation and the audit trail" |

### Read and kept

- **The conformance and performance figures** on the README, the landing page
  and the book: rendered from `docs/conformance/ferroehr/` by
  `scripts/render/conformance-stats.sh` and `scripts/render/perf-assets.sh`,
  and checked in CI. No number is typed.
- **"openEHR-spec-conformant"**, on the [Conformance](../conformance.md) page,
  in the `openehr-*` crate READMEs, in the `org.opencontainers.image.description`
  label of the server image and in the Helm chart description: a claim about
  the openEHR specifications, backed by the committed record (CORE and
  STANDARD, every case passed) and explained on that page. It makes no claim
  about any regulation.
- **"aims to be the first openly developed, source-available openEHR CDR with a
  published, tracker-backed EU compliance posture"** (the
  [compliance overview](index.md), [Why FerroEHR exists](../why-ferroehr.md),
  [Comparison](../comparison.md)): stated as an aim, beside the statement that
  FerroEHR holds no certification and no declaration.
- **The compliance pages, the DPIA and the go-live checklist**: each says it
  does not make a deployment compliant with anything.
- **The image labels**: the server image carries
  `eu.ferroehr.image.carries-sandbox-posture="true"`, which tells an operator
  the shipped profile; the manufacturer label names Cadasto B.V.
- **`SECURITY.md`**: its commitments match the procedures in
  `docs/post-market.md` and the release procedure.
- **The public sandbox**: the viewer shows the sandbox notice on every page,
  the README calls its content demo data, wiped nightly, and the landing page
  calls it a live demo.

### Left open

- **The Helm chart's description** (`Chart.yaml`, and the chart README that
  helm-docs generates from it) says "hardened-by-default security posture"
  and lists the workload hardening, which is accurate, but does not say that
  a stock install runs the `sandbox` profile. The chart now declares
  `config.deployment_profile: sandbox` in its values, and the chart README
  says so under "Before you install"; the `Chart.yaml` description still does
  not.

## The review of 2026-10-08

Read on the tree after commit `de648fcc3`, for FerroEHR 4.3.6. This release
removes the in-tree FHIR connector (#3080) and moves the deployment gaps off
the public status document (#3671), so those two facts were checked in every
text in scope.

### Changed on 2026-10-08

| # | Text | Claim | Art. 28 | Finding | Now reads |
|---|---|---|---|---|---|
| 1 | Landing page, "Fits your stack" | "FHIR R4B" | (a) | FerroEHR maps no FHIR resources; it reaches FHIR R4B terminology servers and renders its access log as FHIR R4 `AuditEvent` | "FHIR R4B terminology" |
| 2 | Book, [instructions for use](instructions-for-use.md), [CRA user information](cra-user-information.md), [configuration](../installation/configuration.md#deployment_profile), [go-live checklist](../security/go-live-checklist.md) | an accepted deployment gap is stated "on `GET /ferroehr/rest/status`" | (a) | the gaps moved to the authenticated `GET /management/status` (#3671); the public status document carries the profile only | "on `GET /management/status`" |
| 3 | Book, [instructions for use](instructions-for-use.md), the deployment profile | the list of what `production` refuses | (b) | omitted the plaintext audit feed and the plaintext object store (#3676, #3689) | both named |
| 4 | Book, [instructions for use](instructions-for-use.md), limitations | none on FHIR | (b) | an operator of an earlier release may rely on the removed connector | FerroEHR maps no FHIR resources, and `[fhir]` is refused at boot |
| 5 | Book, [information sheet](information-sheet.md), (e) | the standards table | (b) | omitted the FHIR terminology operations and RFC 9470, which this release uses | both listed |

### Read and kept on 2026-10-08

- **The README's integration list and the book's
  [Beyond the core](../beyond-core/index.md)**: both state that FHIR R4 mapping
  is FerroBRIDGE's and name the two uses of FHIR FerroEHR keeps.
- **Single tenancy**: the README, the landing page and the book state one
  instance per organisation; "multi-tenant" appears only where the book quotes
  openEHR's placement of multi-tenancy outside one system.
- **The Helm chart's description**: it now says a stock install runs the
  `sandbox` profile (#3637), which closes the item left open on 2026-10-06.

### Left open on 2026-10-08

Nothing.

## The review of 2026-10-15

Read on the tree after commit `606a29cce`, for FerroEHR 4.3.9. The facts of
this release checked in every text in scope: the ABAC patient scope over
EHR-only queries and `[authz.abac] patient_namespace` (#3771); a restricted
`EHR_ACCESS` default with the role gate off, and the `rbac_off` gap (#3742);
national identifiers sealed per party version, the sealed form in write
responses and the refusal of plaintext protected values on import (#3772);
digest version 2 of the audit chain (#3686); the AQL row rules (#3733,
#3734); the removal of FOSSA; the TLS certificate reload and EC keys (#3765,
#3761); the four-cluster production layout (#3755 to #3757, #3762); the
example generator (#3739, #3740); quoted GET query parameters (#3781). The
2026-10-08 move of the deployment gaps to `GET /management/status` (#3671) was
re-checked on every page, and eleven pages still named the public document.

### Changed on 2026-10-15

| # | Text | Claim | Art. 28 | Finding | Now reads |
|---|---|---|---|---|---|
| 1 | Book, [licensing](../licensing.md), [security](../security.md), [go-live checklist](../security/go-live-checklist.md) (two items), [Kubernetes](../installation/kubernetes.md), [authentication configuration](../installation/config-auth.md) (two places), [Compose](../installation/compose.md) (two places), [installation](../installation/index.md), [configuration](../installation/configuration.md#deployment_profile), [command line](../installation/config-cli.md), [DPIA](../security/dpia.md) R9, [compliance overview](index.md) (GDPR Art. 25, DSV Art. 4) | an open or accepted deployment gap, or the licence in force, is reported on `GET /ferroehr/rest/status` (or `/rest/status`) | (a) | the public document carries `status`, the versions, the timestamp and the profile only; the gaps, the licence and the support period are on `GET /management/status` (#3671) | "on `GET /management/status`"; the configuration page adds that the public document carries the profile only |
| 2 | Book, [instructions for use](instructions-for-use.md), the deployment profile | the list of what `production` refuses | (b) | omitted the role gate off while authentication is on, the `rbac_off` gap this release adds (#3742) | named |
| 3 | Book, [instructions for use](instructions-for-use.md), limitations | none on how AQL reads repeated nodes | (b) | AQL 1.1.0 does not define the rows of a path through a repeated node; this release changed them (#3733), so another server, or a client written for an earlier release, may count differently | the limitation is stated, with the regrouping a client needs |
| 4 | Book, [instructions for use](instructions-for-use.md), limitations | none on the audit chain's coverage | (b) | records written before the upgrade keep digest version 1, which leaves out the columns added after the chain was introduced (#3686) | stated, with the columns and the trigger that alone protects them |
| 5 | Book, [configuration](../installation/configuration.md#deployment_profile), `migrate_on_runtime_credential` | a domain relocated to its own database "is prepared on that domain's runtime DSN" | (a) | `[storage.migrate.<domain>] url` now prepares it on its own migration credential (#3756); only without it does the runtime DSN prepare it and open the gap | both cases stated |
| 6 | Book, [records of processing](../security/records-of-processing.md), categories of personal data | clinical content "in the `ehr` schema"; parties "in the `demographic` schema" | (a) | the schemas are `clinical` and `party` since the storage rewrite | `clinical` and `party` |
| 7 | Book, [Kubernetes](../installation/kubernetes.md), server TLS | the viewer's OIDC client "trusts the public roots it is built with", so a private-CA issuer is not trusted | (a) | the viewer's OIDC client verifies against the platform trust store (#3767) | a private-CA issuer is trusted once its CA is in the store or under `SSL_CERT_DIR` |
| 8 | Book, [querying with AQL](../querying-aql.md), the URL form | how a parameter value binds | (b) | silent on a quoted value, which now binds the JSON string it denotes (#3781) | the quoted form and the malformed case stated |
| 9 | Book, [templates and validation](../templates-validation.md), the example composition | "a skeleton instance you can fill in", with no limitation | (b) | the example passes the server's own validation except for two open defects (#3779, #3780) and templates whose mandatory content only a slot admits | the exceptions named |
| 10 | Book, [threat model](../threat-model.md), what each database credential can reach | the writer roles hold `SELECT`, `INSERT`, `UPDATE` and `DELETE` on their schema | (b) | the migration bookkeeping is now read-only to every runtime role, and the boot refuses a writable one | stated below the table |
| 11 | Book, [information sheet](information-sheet.md), (e) | TERM "3.1.0" | (b) | the server applies amendment 3.1.4 (SPECPR-450), which adds properties and units the template upload and the quantity validation accept (#3717, #3727) | "3.1.0, with amendment 3.1.4 (SPECPR-450)" |

The technical documentation's Annex III 2 named the public status document for
the deployment posture too; it names `GET /management/status` now.
| 12 | Repository README, the template-model and example bullets (two places) | every template "generates spec-valid example compositions that pass the server's own validation" | (a) | #3779 and #3780 are open, and a template whose mandatory content only an archetype slot admits has no committable example | "a template \[…\] generates" them, "except for the cases the book's example page lists" |
| 13 | Repository README, the deployment posture and licence bullets | `sandbox` names every open gap "on `GET /rest/status`"; "`GET /ferroehr/rest/status` reports the grant in force" | (a) | both are on `GET /management/status` (#3671) | "on `GET /management/status`" |
| 14 | Book, [EHDS readiness](ehds-readiness.md) row 1.2 | its issue reference | (a) | the probe that issue asked for ran (#3692), without `hostUsers: false` or `sslmode=verify-full` | names #3788 for those two |

### Read and kept on 2026-10-15

- **FOSSA**: no text in scope names FOSSA, a FOSSA badge or a FOSSA lane after
  its removal in this release.
- **The patient scope and `patient_namespace`**: the
  [`[authz.abac]`](../installation/config-auth.md#authzabac) table and its
  patient-scope list state the EHR-only queries, the direct-route refusals and
  the namespace rule as they ship; the [security](../security.md) page's
  fail-closed box is unchanged by them.
- **The restricted `EHR_ACCESS` default and `rbac_off`**: the authentication
  configuration, the configuration page, the go-live checklist, the Kubernetes
  values table and the [CRA risk assessment](cra-risk-assessment.md) (d) state
  both.
- **Identifier protection**: the privacy configuration, the threat model's
  sealed-identifier section, the operations guide and the admin API page state
  the per-version rows, the sealed write response, the plaintext refusal on
  import and load, and the erasure of sealed rows.
- **The audit chain**: the [audit trail](../audit.md#tamper-evidence) page
  states both digest versions and the downgrade finding.
- **AQL**: [querying with AQL](../querying-aql.md) and the AQL engine concept
  page state the row rules and say AQL 1.1.0 does not define them.
- **TLS**: the server configuration and the Kubernetes page state the reload
  interval, the EC key formats and the explicit-curve refusal.
- **The information sheet's BALP row** ("the profile revision is not pinned"):
  kept. The delete agent types follow BALP 1.1.4, and the profile URLs the
  records carry are unversioned.
- **The image labels and the Helm chart's description**: unchanged by this
  release but for the chart description's NetworkPolicy defaults (#3743),
  which match `values.yaml`.

### Left open on 2026-10-15

Nothing.

## The review of 2026-10-16

Read on the tree after commit `e923076de`, for FerroEHR 4.3.10. The facts of
this release checked in every text in scope: the usage report in format 2
(#3795); the usage report off in a CI run, whatever the switch says (#3794);
the dump and load archive on zip 9.0.4 (#3798).

### Changed on 2026-10-16

| # | Text | Claim | Art. 28 | Finding | Now reads |
|---|---|---|---|---|---|
| 1 | Book, [instructions for use](instructions-for-use.md), the usage report | "Every deployment sends a daily usage report to Cadasto B.V. by default" | (a) | the report also goes out at each start, and a server started in a CI run sends none (#3794) | both stated, with the two CI variables |
| 2 | Book, [operations](../operations.md#the-usage-report), the usage report | "It is on by default" | (a) | off in a CI run, whatever the switch says (#3794) | "on by default outside a CI run" |

### Read and kept on 2026-10-16

- **The usage report's fields**: the [usage report](../usage-report.md) page
  lists the format 2 fields and the `status` object as the server sends them,
  and says earlier releases send format 1. The field list in the
  [CRA risk assessment](cra-risk-assessment.md)'s outbound data flows, the
  [shared responsibility](shared-responsibility.md) summary, the
  [records of processing](../security/records-of-processing.md) note, the
  [intended purpose](intended-purpose.md) and the
  [threat model](../threat-model.md) name the same information, which format 2
  only regroups.
- **The CI switch**: the [usage report](../usage-report.md#switch-it-off) page
  states the two variables, the boot line and the `--print` note as they ship,
  and the [server configuration](../installation/config-server.md#usage_report)
  table states the two variables.
- **The dump and load archive**: the
  [admin APIs](../operations-admin-apis.md#dump-and-load) page names no zip
  library and its `compression_format` values are unchanged.
- **The README, `SECURITY.md`, the crate READMEs, the image labels and the Helm
  chart's description**: unchanged by this release, and none names the usage
  report's format or the zip library.

### Left open on 2026-10-16

Nothing.

## Annex II 2.5, re-assessed

EHDS Annex II 2.5: "The harmonised software components of an EHR system shall
not include features that prohibit, restrict or place an undue burden on
authorised access, personal electronic health data sharing or use of personal
electronic health data for permitted purposes." The text does not define
"undue".

The [EHDS readiness](ehds-readiness.md) page marks the row "Shipped" with two
pieces of evidence: the query surface over the whole stored record, and an
authorisation layer that refuses or permits without a commercial gate. The
question re-assessed here is whether the authorisation defaults change that,
now that `production` refuses the open `EHR_ACCESS` default unless it is
accepted by name.

- **What "authorised" means here.** The deploying organisation decides who is
  authorised, and expresses it in RBAC roles, ABAC policy and per-EHR
  `EHR_ACCESS` settings. A refusal of a caller the organisation has not
  authorised is not a burden on authorised access.
- **The `restricted` default.** Under `ehr_access_default = "restricted"`, an
  EHR with no settings is reachable only by the admin role, so a professional
  the organisation means to authorise is refused until someone writes that
  EHR's settings. That is a burden, and it is the organisation's choice: the
  `open` default remains available, `production` accepts it by name, and the
  restrictive one exists because GDPR Art. 25(2) asks that personal data are
  not by default "made accessible without the individual's intervention to an
  indefinite number of natural persons". The settings take role entries
  (`role:nurse`), so one entry covers a whole role, and the admin role can
  always reach the record to write them
  ([per-EHR access control](../security.md#per-ehr-access-control-ehr_access)).
- **Restriction of processing.** A record under restriction is refused on every
  read path. The controller sets the mark under GDPR Art. 18; it is a
  restriction the organisation decides on, not one the software imposes
  ([retention, restriction and objection](retention.md)).
- **Small-cell suppression.** A cohort query that would serve fewer distinct
  EHRs than `cohort.small_cell_threshold` (default 5) withholds its rows and
  says so. It applies only to population queries across the pseudonymisation
  boundary, and the deployment sets the threshold.
- **Rate limits and the admission limit.** On by default, set above the
  server's own measured whole-server ceiling, so they refuse abuse and leave
  normal load alone ([request limits](../security.md#request-limits-and-rate-limiting)).
- **The licence.** The licence in force changes only the stamp on new
  identifiers; it gates no feature, endpoint, limit or message
  (`app/ferroehr/src/licence/mod.rs`).

**Outcome.** No feature found prohibits or restricts access, sharing or use
that the deploying organisation has authorised, and the burdens found are
settings the organisation chooses and can change. The "Shipped" status stands
on this reasoning, which the readiness page will cite.

## Review record

One row per release. The release procedure adds the row when the release is
cut, after the texts in scope have been read.

| Reviewed | Tree | Changed | Left open |
|---|---|---|---|
| 2026-10-06 | before 4.3.5, after `d6539d470` | 15 claims, in the README, the landing page and three book pages | the chart description (#3637) |
| 2026-10-08 | 4.3.6, after `de648fcc3` | 5 claims, in the landing page and five book pages; the chart description item closed | nothing |
| 2026-10-15 | 4.3.9, after `606a29cce` | 14 claims, in 17 book pages, the README and the readiness row | nothing |
| 2026-10-16 | 4.3.10, after `e923076de` | 2 claims, in two book pages | nothing |
