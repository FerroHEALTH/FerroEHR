# Technical documentation of the FerroEHR part of the EHR system

The technical documentation Cadasto B.V., the manufacturer of each tagged
FerroEHR release, keeps under Regulation (EU) 2025/327 (the EHDS,
`docs/law/eu/ehds/text.html`) Art. 37 and Annex III, and, from 11 December
2027, under Regulation (EU) 2024/2847 (the CRA, `docs/law/eu/cra/text.html`)
Art. 31 and Annex VII. EHDS Art. 104(2) replaces CRA Art. 31(3) so that a
product classified as an EHR system draws up "a single set of technical
documentation" for both acts, and EHDS Art. 104(3) inserts CRA Art. 32(5a),
which routes the CRA's Annex I conformity through the EHDS Chapter III
procedure. The position is recorded in
[`docs/architecture.md`](../architecture.md#regulatory-position-ehds-and-cra).

The EHDS Annex III elements describe the EHR system FerroEHR forms with
FerroBRIDGE, so they live in the FerroHEALTH book
(<https://ferrohealth.eu/docs/ehds/technical-documentation.html>, one page per
element), and each FerroEHR release relies on the revision of those pages that
[`docs/ehds/system-documents.env`](../ehds/system-documents.env) pins, named
by the FerroHEALTH commit and the date on each page. This tree keeps
FerroEHR's own CRA Annex VII points and this index. The tree and the pin as
they stand at a release tag are the documentation of that release. No
conformity assessment has been carried out and no EU declaration of
conformity exists; several elements below are incomplete, and each file says
what is missing.

## Scope

The EHR system Cadasto B.V. declares is FerroEHR and FerroBRIDGE together
(EHDS Art. 2(2)(k), Art. 25(1)). This tree covers FerroEHR's part:

- the European logging software component (EHDS Art. 2(2)(o), Annex II 3),
  which in the code is the FerroHEALTH products' shared logging crate
  `ehds-logging` and audit crate `ihe-atna`, both 0.3.1 from FerroAUDIT
  (<https://github.com/FerroHEALTH/FerroAUDIT>, pinned exactly in the root
  `Cargo.toml`), and `ferroehr::system_log` (with
  `system_log::chain_check`, the scheduled verification of the hash chain),
  the audit schema migrations in `app/ferroehr/migrations/audit/`,
  `ferroehr-rest::system_log`, the ITI-81 `GET /fhir/r4/AuditEvent` route in
  `ferroehr-rest::extensions::fhir`, and the retention register in
  `ferroehr::storage::marks`;
- the requirements of EHDS Annex II 1 that the system as a whole carries;
- FerroEHR as a product with digital elements under the CRA.

FerroBRIDGE keeps the documentation of the European interoperability software
component (EHDS Art. 2(2)(n), Annex II 2.1 to 2.3) with its own releases. The
two components meet only over the openEHR REST API, which is what keeps them
independent of each other (EHDS Art. 30(1)(b)).

The access log is planned to become a shared FerroHEALTH service in FerroSYS
(#3661). From the release that ships it, the declared EHR system is FerroEHR,
FerroBRIDGE and FerroSYS, and the logging component is split: FerroEHR keeps
the recording at the moment of access (Annex II 3.2), the classification from
its own data and the local spool that refuses an access whose record cannot
be written, and forwards each record to FerroSYS; FerroSYS holds the central
store, the review and analysis tools (Annex II 3.3), the retention per origin
and category (Annex II 3.4), the natural person's access-log route (EHDS Art.
9) and the ITI-81 retrieval. This tree then documents FerroEHR's part, the
documentation of FerroSYS's part covers the rest, and the scope above and the
list in `scripts/checks/technical-documentation.sh` change in the same
release. Both parts stay outside FerroBRIDGE, so the two harmonised components
remain independent ([`docs/architecture.md`](../architecture.md#regulatory-position-ehds-and-cra)).

## Contents

The EHDS Annex III elements, with CRA Annex VII 1(a), (b) and (d), 6 and 7,
which they answer, are pages of the FerroHEALTH book, at the revision the pin
names:

| Requirement | Page |
|---|---|
| EHDS Annex III 1(a) to (j); CRA Annex VII 1(a), (b), (d) | [Annex III 1](https://ferrohealth.eu/docs/ehds/annex-iii-1-description.html) |
| EHDS Annex III 2 | [Annex III 2](https://ferrohealth.eu/docs/ehds/annex-iii-2-performance-evaluation.html) |
| EHDS Annex III 3 | [Annex III 3](https://ferrohealth.eu/docs/ehds/annex-iii-3-common-specifications.html) |
| EHDS Annex III 4; EHDS Art. 37(2), the testing-environment results; CRA Annex VII 6 | [Annex III 4](https://ferrohealth.eu/docs/ehds/annex-iii-4-verification-and-validation.html) |
| EHDS Annex III 5 | [Annex III 5](https://ferrohealth.eu/docs/ehds/annex-iii-5-information-sheet.html) |
| EHDS Annex III 6; CRA Annex VII 7 | [Annex III 6](https://ferrohealth.eu/docs/ehds/annex-iii-6-declaration-of-conformity.html) |

The files of this tree:

| Requirement | File |
|---|---|
| CRA Annex VII 2 and 8 | [`cra-annex-vii-2-design-and-vulnerability-handling.md`](cra-annex-vii-2-design-and-vulnerability-handling.md) |
| CRA Annex VII 3 | [`cra-annex-vii-3-risk-assessment.md`](cra-annex-vii-3-risk-assessment.md) |
| CRA Annex VII 4 | [`cra-annex-vii-4-support-period.md`](cra-annex-vii-4-support-period.md) |
| CRA Annex VII 5 | [`cra-annex-vii-5-standards-and-solutions.md`](cra-annex-vii-5-standards-and-solutions.md) |

CRA Annex VII 1(c), photographs of a hardware product, does not apply:
FerroEHR is software.

`scripts/checks/technical-documentation.sh` checks that every file in this
table exists and that every relative link in the tree resolves.

## Keeping it up to date (EHDS Art. 30(2), Art. 37(1); CRA Art. 31(2))

EHDS Art. 30(2): "Changes in EHR system design or characteristics with regard
to the harmonised software components of an EHR system shall be adequately
taken into account and reflected in the technical documentation." A release
whose diff since the previous release touches a module of the logging
component (the paths in `docs/architecture.md`, and the same list in
`scripts/checks/technical-documentation.sh`) updates, in the same release, the
file of this tree the change affects or the FerroHEALTH page it affects (and
moves the pin to that revision), and adds a row to the revision table below.
`scripts/checks/technical-documentation.sh --since <previous tag>` refuses a
release that changes those paths and leaves both the tree and the pin
untouched; the release procedure in `.claude/rules/changelog.md` runs it.

## Keeping it for ten years (EHDS Art. 30(3); CRA Art. 13(13))

EHDS Art. 30(3) has the manufacturer keep the technical documentation and the
EU declaration of conformity "for 10 years after the EHR system covered by the
EU declaration of conformity has been placed on the market". CRA Art. 13(13)
asks for "at least 10 years after the product with digital elements has been
placed on the market or for the support period, whichever is longer". Each
release's documentation is kept in three places, none of which is rewritten:

- the signed release tag `vX.Y.Z`, which the `release-tags` ruleset protects
  against deletion and against being moved (`SECURITY.md`, repository
  settings);
- the release's archive on Zenodo, which receives the source tree of each
  published release (the concept DOI is in `README.md` and `CITATION.cff`);
- the book frozen for each release at `https://ferroehr.eu/docs/vX.Y.Z/`, on
  the `docs-dist` branch, which carries the pages this tree cites at that
  release.

The FerroHEALTH book keeps every revision of the Annex III pages in its
history, and a release names the revision it relies on in
`docs/ehds/system-documents.env` and in its notes. The information sheet and
the instructions for use of each release are attached to its GitHub release
as assets, with that release's facts appended, and GitHub releases are
immutable.

## Translation on request (EHDS Art. 37(3) and (4))

The documentation is written in English. EHDS Art. 37(3) has the manufacturer
provide, "following a reasoned request from the market surveillance authority
of a Member State", a translation of the relevant parts into an official
language of that Member State, and Art. 37(4) sets 30 days from the request,
"unless a shorter deadline is justified because of a serious and immediate
risk". The request goes to Cadasto B.V.'s single point of contact,
info@cadasto.com. Cadasto B.V. commissions the translation of the parts the
request names, sends it within the deadline, and keeps the request and the
answer outside the repository, as for any request from an authority
(<https://ferrohealth.eu/docs/post-market.html>). EHDS
Art. 30(3) second subparagraph adds the source code on a reasoned request: the
source of every release is public at its tag.

## Open owner facts

- **The certified scope.** Cadasto B.V. holds ISO 9001, ISO/IEC 27001 and
  NEN 7510 certification. A certificate covers Cadasto B.V.'s management
  system within its certified scope; it never covers the product, and nothing
  in this tree calls FerroEHR certified. Whether the scope covers the
  development and release of FerroEHR and a FerroEHR service Cadasto B.V.
  hosts is not confirmed, so no file here relies on the certificates
  ([`docs/post-market.md`](../post-market.md#facts-only-the-owner-supplies),
  item 9).
- **A minimum hardware specification** for
  [Annex III 1(f)](https://ferrohealth.eu/docs/ehds/annex-iii-1-description.html#1f-the-hardware-it-is-intended-to-run-on).

## Revisions

| Date | Release | Change |
|---|---|---|
| 2026-10-06 | before 4.3.5 | First version of the tree (#3616) |
| 2026-10-08 | 4.3.6 | The access record gains the acting mode, the assurance level and the professional behind a client (#3622, audit migration 0009) and the emergency mark (#3624, audit migration 0010), rendered as the [hazard log](../../website/book/src/compliance/hazard-log.md) H2 and H9 state; every record carries its request id (#3665). The hazard log, cited by Annex III 4, names the tests |
| 2026-10-08 | 4.3.6 | The in-tree FHIR connector is removed (#3080); the ITI-81 `GET /fhir/r4/AuditEvent` route is unchanged and is now the only route in `ferroehr-rest::extensions::fhir` |
| 2026-10-08 | 4.3.6 | The family-level documentation moved to the FerroHEALTH book at <https://ferrohealth.eu/docs/> (#3701): the statement of intended purpose for the EHR system, the shared responsibility, the post-market procedure and the manufacturer side of the CRA; 1(a) and the vulnerability-handling policy of CRA Annex VII 2 now cite it beside FerroEHR's own pages |
| 2026-10-15 | 4.3.9 | The tamper chain's digest version 2 covers every column of an access record (#3686, audit migration 0011); records written before keep verifying under version 1, and a record moved back to it is reported. The DICOM and FHIR renderings give the client and the server one data-flow role (#3687), and a FHIR delete record carries the BALP `Delete` agent types (#3750). The physical party delete is classified and names its party (#3683). The audit store and the retention register take every connection through the typed acquire, so a connect failure is an `audit store unavailable` error (#3749). [Annex III 4](https://ferrohealth.eu/docs/ehds/annex-iii-4-verification-and-validation.html) names the new tests, and the [hazard log](../../website/book/src/compliance/hazard-log.md) H1, H2 and H7 state the changes; [Annex III 2](https://ferrohealth.eu/docs/ehds/annex-iii-2-performance-evaluation.html) names `GET /management/status` for the deployment posture |
| 2026-10-19 | 4.3.11 | The scope names the planned split of the logging component with FerroSYS and the three-product EHR system that follows (#3662); 1(a) and Annex III 6 name FerroSYS as the third product from that release |
| 2026-10-20 | 5.1.0 | The audit FHIR feed authenticates with an OAuth 2.0 client-credentials token, renewed before it expires and fetched again once on a `401`; an unreachable token endpoint leaves the records pending as an unreachable feed does (#3829). The assurance and professional rules the access record reads move under `[authz]` with the move to the shared authentication crate (#3872). [CRA Annex VII 5](cra-annex-vii-5-standards-and-solutions.md) names RFC 9068, RFC 8414, the client-credentials grant and the SMART on openEHR scopes; the [hazard log](../../website/book/src/compliance/hazard-log.md) is reviewed and every test it names still exists |
| 2026-10-20 | after 5.2.0 | The logging component takes its access record, its categories and their classification, the Art. 9(2) retention floor and the emergency mark from the shared crate `ehds-logging` 0.1.1, and its BALP and DICOM renderings and its ATNA sender from `ihe-atna` 0.1.1 (FerroAUDIT, #3887); FerroEHR's own copies are removed. A record stores each category as its HL7 Europe `<system>\|<code>` token and says what its classification was read from (audit migration 0012, clinical migration 0016); the syslog feed sends over RFC 5425 TLS only. The [hazard log](../../website/book/src/compliance/hazard-log.md) is reviewed and names the moved tests; [Annex III 4](https://ferrohealth.eu/docs/ehds/annex-iii-4-verification-and-validation.html) names where the crates' own tests run |
| 2026-10-20 | 5.2.0 | The access log gains the records of two new reads: a read of a party's protected national identifiers, one record per scheme naming the reader, the party and the scheme (#3776), and the restriction register's own acts (#3682). The access context carries whether a request enforces the EHDS Art. 8 restriction, and an emergency access that overrides it carries the emergency mark (#3682). The [hazard log](../../website/book/src/compliance/hazard-log.md) H9 names the restriction and its test |
| 2026-10-22 | 5.3.2 | The logging component moves to FerroAUDIT 0.2.4 (`ehds-logging` and `ihe-atna`, #3902): the record writes again the token's `jti`, the ITS-REST operation id as a second `subtype`, `outcomeDesc`, `source.type` and the optional DICOM elements; a Basic user is recorded with no issuer and no client; a retention without end or counted in days is stated as such; the category map takes every archetype id openEHR BASE 1.3.0 admits and keeps each id as the access named it, a stored composition's root archetype in the node table's comparison form (#3908). The ITI-81 search form-decodes its query string, so a `+` is a space (#3891). The component description names the 0.2.4 pins, and the [hazard log](../../website/book/src/compliance/hazard-log.md) is reviewed and every test it names still exists |
| 2026-10-24 | 5.4.0 | The logging component moves to FerroAUDIT 0.2.5 (`ehds-logging` and `ihe-atna`, #3959): when the audit drain exits it closes its syslog connection with a TLS `close_notify` (RFC 5425 §4.4), so the records it sent last are no longer lost to a connection reset, and it logs and counts a close that fails; the syslog connection sends each record at once. The component description names the 0.2.5 pins, and the [hazard log](../../website/book/src/compliance/hazard-log.md) is reviewed, H5 names the close test and every test it names still exists |
| 2026-10-26 | after 5.4.2 | The EHDS Annex III elements, the information sheet, the instructions for use and the readiness record move to the FerroHEALTH book as the EHR system's documents (#3705); this tree keeps FerroEHR's CRA Annex VII points, and `docs/ehds/system-documents.env` pins the revision of the family pages each release relies on |
| 2026-10-28 | 5.6.0-rc1 | The audit store keeps each record's facts in columns and renders its `AuditEvent` from them when the record is read over ITI-81 or fed, under the render policy the record names (`audit.audit_render_policy`); the chain digest covers every column in one version, and audit migration 0017 refuses a store holding records of an earlier release (#4300). The three `recorded_at` indexes ascend (#4302), and every ITI-81 parameter answers through indexed search tokens the chain covers (#4313). The ITI-81 retrieval moves to `/ferroehr/rest/fhir/r4/AuditEvent` (#4330). A deployment that declares no `[audit.categories]` template or archetype classifies through the category map FerroEHR ships for the published CKM templates and archetypes of one priority category (#4298). The component description names the 0.3.1 pins, and the [hazard log](../../website/book/src/compliance/hazard-log.md) is reviewed, H3 names the shipped map's test, H7 the render-policy and search-token tests, and every test it names exists |
| 2026-10-29 | 5.6.0-rc2 | 5.6.0-rc1 was tagged and not published, so 5.6.0-rc2 carries its row above. Since then: a body without `FEEDER_AUDIT` committed directly by a client the `[origins]` register names records that client's origin, and the retention register in `ferroehr::storage::marks` keys a period on a registered origin beside the category (clinical migration 0020, #4299). The [hazard log](../../website/book/src/compliance/hazard-log.md) is reviewed, H4 names the origin of a direct commit and its test, and every test it names exists |
