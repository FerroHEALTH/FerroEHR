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

## The review of 2026-10-19

Read on the tree after commit `a15290689`, for FerroEHR 4.3.11. The facts of
this release checked in every text in scope: the viewer drawn in FerroUI
0.1.6 (#3802, #3811, #3815, #3820); the viewer's tab icon (#3808); the coming
split of the logging component with FerroSYS (#3662); the zip advisories
judged not affecting in the Rust-dependency VEX document (#3806).

### Changed on 2026-10-19

Nothing. Every text in scope already states these facts.

### Read and kept on 2026-10-19

- **The viewer pages**: [the viewer](../viewer/index.md) names the footer's
  manufacturer line and the System screen's Manufacturer card (#3634), and
  [dark mode](../viewer/preferences.md) describes the moon and sun button and
  the theme cookie as FerroUI draws them. No page names a component library
  the viewer no longer uses.
- **The logging component**: the [EHDS readiness](ehds-readiness.md) page and
  the technical documentation say FerroEHR ships the whole logging component
  today and that FerroSYS takes the central store, review, retention and
  retrieval in a later release; nothing calls that split shipped.
- **The security notices**: the release's Security entry names the VEX
  document that records the three zip advisories as not affecting FerroEHR.

### Left open on 2026-10-19

Nothing.

## The review of 2026-10-19, for 5.0.0

Read on the tree after commit `ec78e7f40`, for FerroEHR 5.0.0. The facts of
this release checked in every text in scope: `openehr-sdt` builds a canonical
PARTY from a party Web Template and stamps the caller's `rm_version` (#3810,
#3809); the openEHR terminology serves the `interoperability` group; archetype
identifiers follow the BASE 1.3.0 grammar under the `development` profile
(#3835, #3845); the nine `openehr-*` crates at `0.0.89`; only the newest
release is supported (#3853); the viewer serves its hydration bundle on a
route of its own (#3849); the viewer draws with FerroUI 0.1.7 and chooses its
language from `Accept-Language` or `[ui] locale`.

### Changed on 2026-10-19, for 5.0.0

- **[The Rust crates](../crates.md)**: the dependency examples named the
  crates at `0.0.86`, three versions behind the published line; they name
  `0.0.89`. The `openehr-sdt` row names the PARTY build beside the
  composition formats.
- **[The viewer](../viewer/index.md)**: the `ui.locale` row said a
  deployment adds a language as a catalog file. The catalogs are embedded
  when the viewer is built, so the row now says a new catalog reaches a
  deployment in a rebuilt image, never at run time.

### Read and kept on 2026-10-19, for 5.0.0

- **The support period**: `SECURITY.md`, [the Rust crates](../crates.md) page
  and the release notes say that only the newest release is supported and
  that the five years of security fixes are a commitment for the product,
  shipped in the newest release. No text in scope still promises fixes to an
  earlier release.
- **The crate READMEs**: the `openehr-sdt` README describes the composition
  formats and does not mention the PARTY build. An omission ascribes no
  function the crate lacks, and the README changes with the crate's next
  content version.
- **The specification pins**: `docs/VERSIONS.md` and the crate table name
  BASE 1.2.0 and 1.3.0, which is what the two generations implement.

### Left open on 2026-10-19, for 5.0.0

Nothing.

## The review of 2026-10-20, for 5.1.0

Read on the tree after commit `18097c6f1`, for FerroEHR 5.1.0. The facts of this
release checked in every text in scope: authentication and the SMART on openEHR
scope check run on the shared `ferrohealth-smart` crate, only asymmetric
algorithms verify a token, the HMAC key is gone, and every route requires a
scope (#3872); the Keycloak overlay and the development stack serve their
issuer over TLS with a development CA, and the CDR accepts their tokens again
(#3873); AQL rooted at `PERSON`, `ORGANISATION` and `ROLE`, and a demographic
root joined to its EHR's content (#3818, #3831); the audit FHIR feed and the
remote policy service authenticate with OAuth 2.0 client credentials (#3829);
the chart mounts a database CA for `sslmode=verify-full` (#3788); a rotated
mTLS client CA is re-read without a restart (#3769); the viewer reads every
word from its catalog with FerroUI 0.2.0 (#3860); the nine `openehr-*` crates
at `0.0.90`.

### Changed on 2026-10-20, for 5.1.0

- **The repository `README.md`**: the authentication bullet named Basic and
  OAuth2/OIDC alone. Every route now refuses a caller whose scopes do not
  grant the operation, a limitation a deployment must know (Art. 28(b)), so the
  bullet says the SMART on openEHR scopes are checked on every route.
- **[The Rust crates](../crates.md)**: the dependency examples named the crates
  at `0.0.89`; they name `0.0.90`.

### Read and kept on 2026-10-20, for 5.1.0

- **The Keycloak quickstart**: [Docker Compose](../installation/compose.md),
  [getting started](../getting-started.md) and the README offer the overlay
  for bearer-token authentication. Between #3872 and #3873 the CDR did not
  accept the overlay's tokens; on this tree it does, and the compose page
  names the development CA and the scopes the realm puts in every token.
- **Authentication and authorization**: [security](../security.md),
  [the authentication reference](../installation/config-auth.md) and
  [SMART App Launch](../smart-app-launch.md) describe the `[auth]` keys, the
  RFC 9068 profile and the scope check as the code does.
- **Annex II 2.5**: a caller refused for want of a scope is refused by the
  grant its own identity provider issued, which the deployment controls; the
  re-assessment below stands.

### Left open on 2026-10-20, for 5.1.0

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
| 2026-10-19 | 4.3.11, after `a15290689` | none | nothing |
| 2026-10-19 | 5.0.0, after `ec78e7f40` | 2 claims, in two book pages | nothing |
| 2026-10-20 | 5.1.0, after `18097c6f1` | 2 claims, in the README and one book page | nothing |
| 2026-10-20 | 5.2.0, after `e5aa1902b` | 1 claim, in one book page | nothing |
| 2026-10-22 | 5.3.2, after `434cd351e` | 4 claims, in four book pages and the technical documentation | nothing |
| 2026-10-23 | 5.3.3, after `25845e6d3` | 1 claim, in one book page: the templates page said `|raw` is write-only, corrected for #3934 | nothing |
| 2026-10-24 | 5.4.0, after `de38ec433` | none | nothing |
| 2026-10-24 | 5.4.1, after `9fdd36091` | 2 claims, in two book pages: the compliance index named `[terminology.external]` and a terminology route, and the crates page pinned `0.0.97` | nothing |
| 2026-10-25 | 5.4.2, after `2333f98ba` | none; the changes of this release updated their own pages in the pull requests that made them | nothing |

## The review of 2026-10-20, for 5.2.0

Read on the tree after commit `e5aa1902b`, for FerroEHR 5.2.0. The facts of this
release checked in every text in scope: authentication runs on
`ferrohealth-smart` 0.3.1, with `[auth.basic] cache_ttl_seconds` in place of
`[authz] basic_cache_ttl_seconds`, `scopeless_subjects` accepted and a
malformed bearer token answered `401` (#3875); `[smart] issuer_metadata_file`
(#3875); the EHDS Art. 8 restriction of access with its emergency override
(#3682); the audited read of a party's protected identifiers (#3776); an EHR
extract never carries a sealed identifier (#3777); the FLAT history origin from
the earliest event (#3888); the nine `openehr-*` crates at `0.0.91`.

### Changed on 2026-10-20, for 5.2.0

- **[The Rust crates](../crates.md)**: the dependency examples named the crates
  at `0.0.90`; they name `0.0.91`.

### Read and kept on 2026-10-20, for 5.2.0

- **The Art. 8 statements**: [security](../security.md), the
  [audit trail](../audit.md), the [compliance overview](index.md), the
  [instructions for use](instructions-for-use.md), [EHDS readiness](ehds-readiness.md)
  rows 1.1 and 1.3 and the [hazard log](hazard-log.md) describe the restriction
  as the code does, and none of them calls it planned any more.
- **The authentication reference**: [the configuration reference](../installation/config-auth.md),
  [security](../security.md) and [using the API](../using-the-api/index.md)
  name the moved cache key and the `401` for a bearer value outside `b64token`.

### Left open on 2026-10-20, for 5.2.0

Nothing.

## The review of 2026-10-22, for 5.3.2

Read on the tree after commit `434cd351e`, for FerroEHR 5.3.2. The facts of this
release checked in every text in scope: the access log written through
FerroAUDIT's `ehds-logging` and `ihe-atna` 0.2.4, with the categories named by
their HL7 Europe codes, `category_basis`, and the record's `jti`, operation
subtype and `outcomeDesc` written again (#3887, #3902); the retention register
keyed on the category token (clinical migration 0016); the syslog feed over
RFC 5425 TLS only, with the UDP transport removed; `ferrohealth-smart` 0.4.0
and one login record per Basic credential verification (#3895); a `+` in a
query string read as a space (#3891); the subject id of a lookup by subject
kept out of every log, span and error body (#3906); Rust 1.99.1 with the MSRV
at 1.98; the nine `openehr-*` crates at `0.0.95`.

### Changed on 2026-10-22, for 5.3.2

- **[The Rust crates](../crates.md)**: the dependency examples named the crates
  at `0.0.91`; they name `0.0.95`.
- **[EHDS readiness](ehds-readiness.md)**, the European logging component row
  (from `website/book/ehds.yaml`): it named `ehds-logging` and `ihe-atna` at
  0.1.1; it names 0.2.4, the version the root `Cargo.toml` pins.
- **[The hazard log](hazard-log.md)**, the component paragraph: as the row
  above.
- **The technical documentation's scope** (`docs/technical-documentation/README.md`):
  as the row above.

### Read and kept on 2026-10-22, for 5.3.2

- **The audit statements**: the README, [security](../security.md), the
  [audit trail](../audit.md), [audit configuration](../installation/config-audit.md),
  the [go-live checklist](../security/go-live-checklist.md), the
  [information sheet](information-sheet.md) and the
  [instructions for use](instructions-for-use.md) describe the syslog feed as
  TLS only and the categories by their HL7 Europe codes, as the code does.
- **The toolchain**: the README and [building from source](../installation/from-source.md)
  name the Rust 1.99 toolchain, and [the Rust crates](../crates.md) the MSRV 1.98.
- **The landing page, `SECURITY.md`, the crate READMEs, the image labels and
  the chart description** make no claim this release changes.

### Left open on 2026-10-22, for 5.3.2

Nothing.

## The review of 2026-10-24, for 5.4.0

Read on the tree after commit `de38ec433`, for FerroEHR 5.4.0. The facts of this
release checked in every text in scope: the viewer rebuilt on FerroUI 0.3.6 in
islands mode, working with JavaScript off, with its new addresses, its
preferences as cookies of their own and an open tab that idles out (#3932,
#3057); an ECDSA or Ed448 OpenPGP signing key refused at boot for
RUSTSEC-2026-0387 (#3965); the signer's choice of subkey and primary key
(#3967, #3969, #3971); the audit drain closing its syslog connection with
`close_notify` and FerroAUDIT 0.2.5 (#3959); the nine `openehr-*` crates
unchanged at `0.0.97`.

### Changed on 2026-10-24, for 5.4.0

Nothing: the pull requests that made these changes updated the texts they
touched.

### Read and kept on 2026-10-24, for 5.4.0

- **The viewer statements**: the README, the [viewer](../viewer/index.md)
  pages and the image label of `docker/viewer/Dockerfile` describe the viewer
  the code draws; no text names a removed address, the `ferroehr-theme`
  cookie, the Tailwind build or a background poll.
- **The signing statements**: [PGP signing](../signing/pgp.md) and the
  [authentication and signing configuration](../installation/config-auth.md)
  state the refused ECDSA and Ed448 keys, the subkey the server signs with and
  the retired-key path, as the code does.
- **The audit statements**: the [hazard log](hazard-log.md) and
  [EHDS readiness](ehds-readiness.md) name `ehds-logging` and `ihe-atna`
  0.2.5, the version the root `Cargo.toml` pins.
- **The landing page, `SECURITY.md`, the crate READMEs, the
  [information sheet](information-sheet.md), the
  [instructions for use](instructions-for-use.md), the other image labels and
  the chart description** make no claim this release changes.

### Left open on 2026-10-24, for 5.4.0

Nothing.

## The review of 2026-10-24, for 5.4.1

Read on the tree after commit `9fdd36091`, for FerroEHR 5.4.1. The facts of this
release checked in every text in scope: the CDR asks one terminology server
under `[terminology.server]` through `ferrobridge-term`, and chart 14.0.0 wires
it (#3085); an unserved method is `405` with `Allow` under SMART (#3987);
the conformance statement declares `unmatched-member-refused` (#3988); a ROLE
template whose single attributes state `0..*` uploads, and a demographic
template's example is a party (#3994, #3995); the viewer offers demographic
templates only where they work (#3997); the nine `openehr-*` crates at `0.0.98`.

### Changed on 2026-10-24, for 5.4.1

- **The compliance index** (SGB V § 355 row) named `[terminology.external]`
  and "the terminology route"; it now names `[terminology.server]` and its
  `url`.
- **The crates page** pinned the `openehr-*` crates at `0.0.97`; this release
  publishes `0.0.98`.

### Read and kept on 2026-10-24, for 5.4.1

- **The terminology statements**: the
  [integrations configuration](../installation/config-integrations.md), the
  [Kubernetes installation](../installation/kubernetes.md) and the chart README
  describe the one `[terminology.server]` and `secrets.terminologyClientSecret`,
  as the code and the chart do.
- **The template statements**: the
  [templates page](../templates-validation.md) and
  [content negotiation](../using-the-api/content-negotiation.md) state the
  example party of a demographic template and its JSON and XML forms.
- **The `405` statement**: content negotiation's "`405` always names the
  allowed methods" holds under SMART as well.
- **The landing page, `SECURITY.md`, the crate READMEs, the
  [information sheet](information-sheet.md), the
  [instructions for use](instructions-for-use.md), the image labels and the
  chart description** make no claim this release changes.

### Left open on 2026-10-24, for 5.4.1

Nothing.

## The review of 2026-10-25, for 5.4.2

Read on the tree after commit `2333f98ba`, for FerroEHR 5.4.2. The facts of this
release checked in every text in scope: a valid Certification Revocation of a
Direct Key signature cancels it, so it no longer lets the primary key sign
(#3972); an access record whose patient lookup failed writes `unavailable`
(#3900); the viewer's Paths tab offers a query only on a COMPOSITION template,
on FerroUI 0.3.7 (#4001); `ferrohealth-smart` 0.5.0, one `openehr-sdt` in the
build (#4002); `ihe-atna` and `ehds-logging` 0.2.7 (#3954); zip 8.6.0 out of
the build (#3799); chart 14.0.1; the nine `openehr-*` crates at `0.0.98`.

### Changed on 2026-10-25, for 5.4.2

Nothing at the cut: each change above updated its page in its own pull request.

### Read and kept on 2026-10-25, for 5.4.2

- **The signing statement**: the [PGP signing page](../signing/pgp.md) states
  that a revoked Direct Key signature is not chosen, as the code does.
- **The viewer statement**: [Templates & EHRs](../viewer/browsing.md) says a
  node offers Build a query on this path only on a COMPOSITION template.
- **The logging component**: the [EHDS readiness page](ehds-readiness.md),
  rendered from `website/book/ehds.yaml`, and the [hazard log](hazard-log.md)
  name `ehds-logging` and `ihe-atna` 0.2.7.
- **The installation pins**: the [Kubernetes installation](../installation/kubernetes.md),
  the chart README and [verifying releases](../verifying-releases.md) name chart
  14.0.1 and image 5.4.2.
- **The landing page, `SECURITY.md`, the crate READMEs, the
  [information sheet](information-sheet.md), the
  [instructions for use](instructions-for-use.md), the image labels and the
  chart description** make no claim this release changes.

### Left open on 2026-10-25, for 5.4.2

Nothing.
