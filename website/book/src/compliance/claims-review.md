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
| 7 | README, "Integration" | the FHIR R4B connectors, with no limitation | (b) | the in-tree connector is to be removed in favour of FerroBRIDGE (#3080) | the planned removal is stated |
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
  a stock install runs the `sandbox` profile. That changes when the chart
  declares `deployment_profile` (planned, #3637).

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
