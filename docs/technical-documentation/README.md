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

The tree as it stands at a release tag is the documentation of that release.
No conformity assessment has been carried out and no EU declaration of
conformity exists; several elements below are incomplete, and each file says
what is missing.

## Scope

The EHR system Cadasto B.V. declares is FerroEHR and FerroBRIDGE together
(EHDS Art. 2(2)(k), Art. 25(1)). This tree covers FerroEHR's part:

- the European logging software component (EHDS Art. 2(2)(o), Annex II 3),
  which in the code is `ferroehr::system_log` (with
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

## Contents

| Requirement | File |
|---|---|
| EHDS Annex III 1(a) to (j); CRA Annex VII 1(a), (b), (d) | [`annex-iii-1-description.md`](annex-iii-1-description.md) |
| EHDS Annex III 2 | [`annex-iii-2-performance-evaluation.md`](annex-iii-2-performance-evaluation.md) |
| EHDS Annex III 3 | [`annex-iii-3-common-specifications.md`](annex-iii-3-common-specifications.md) |
| EHDS Annex III 4; EHDS Art. 37(2), the testing-environment results; CRA Annex VII 6 | [`annex-iii-4-verification-and-validation.md`](annex-iii-4-verification-and-validation.md) |
| EHDS Annex III 5 | [`annex-iii-5-information-sheet.md`](annex-iii-5-information-sheet.md) |
| EHDS Annex III 6; CRA Annex VII 7 | [`annex-iii-6-declaration-of-conformity.md`](annex-iii-6-declaration-of-conformity.md) |
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
`scripts/checks/technical-documentation.sh`) updates this tree in the same
release: the file the change affects, and a row in the revision table below.
`scripts/checks/technical-documentation.sh --since <previous tag>` refuses a
release that changes those paths and leaves the tree untouched; the release
procedure in `.claude/rules/changelog.md` runs it.

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

The information sheet and the instructions for use of each release are also
attached to its GitHub release as assets, and GitHub releases are immutable.

## Translation on request (EHDS Art. 37(3) and (4))

The documentation is written in English. EHDS Art. 37(3) has the manufacturer
provide, "following a reasoned request from the market surveillance authority
of a Member State", a translation of the relevant parts into an official
language of that Member State, and Art. 37(4) sets 30 days from the request,
"unless a shorter deadline is justified because of a serious and immediate
risk". The request goes to Cadasto B.V.'s single point of contact,
info@cadasto.com. Cadasto B.V. commissions the translation of the parts the
request names, sends it within the deadline, and keeps the request and the
answer outside the repository, as for any
[request from an authority](../post-market.md#a-request-from-an-authority). EHDS
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
  [Annex III 1(f)](annex-iii-1-description.md#1f-the-hardware-it-is-intended-to-run-on).

## Revisions

| Date | Release | Change |
|---|---|---|
| 2026-10-06 | before 4.3.5 | First version of the tree (#3616) |
