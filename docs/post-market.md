# Post-market procedures

The working checklist for FerroEHR's side of the post-market procedure that
Cadasto B.V., the manufacturer of each tagged FerroEHR release, runs for every
FerroHEALTH product. The procedure itself (the channels, how a report is
classified, the serious-incident report under EHDS Art 44(7), the CRA Art 14
notification, NIS2, when an advisory's details may wait, reporting to the
maintainer of an integrated component, a request from an authority, and the
cessation of operations) is the manufacturer's and is published in the
FerroHEALTH book:
[Post-market](https://ferrohealth.eu/docs/post-market.html). The duties come
from Regulation (EU) 2025/327, the EHDS (`docs/law/eu/ehds/text.html`), and
Regulation (EU) 2024/2847, the CRA (`docs/law/eu/cra/text.html`).

This file keeps the steps bound to this repository: its registers, its
tracker, its release lane and `scripts/release/withdraw.sh`. The public
account for FerroEHR users is the book's
[Complaints, incidents and vulnerabilities](https://ferroehr.eu/docs/latest/compliance/post-market.html)
page.

## Who does what in this repository

| Who | What |
|---|---|
| Cadasto B.V., the manufacturer | Receives complaints and incident reports at `info@cadasto.com`, decides whether an event is a serious incident or an actively exploited vulnerability, and signs every report to an authority (the FerroHEALTH procedure). |
| The maintainer, Ruben Talstra | Reads FerroEHR's private vulnerability reports, the reports Cadasto B.V. forwards from `info@cadasto.com`, and the FerroEHR tracker; enters the register rows below, prepares the correcting release, runs `scripts/release/withdraw.sh`, and drafts the advisory and the reports for Cadasto B.V. to send. |
| The deploying organisation | Supplies the facts of its deployment and the JSON document `ferroehr report` writes. |

FerroEHR's channels are GitHub private vulnerability reporting on this
repository ([`SECURITY.md`](../SECURITY.md)) and `info@cadasto.com` with the
subject "FerroEHR vulnerability", "FerroEHR incident" or "FerroEHR complaint",
or a public GitHub issue for a complaint that can be public. An emailed
vulnerability report gets a draft security advisory on this repository on
receipt, exactly as a private GitHub report does, and the clocks run from the
email's arrival.

## The registers

- [`registers/complaints.tsv`](registers/complaints.tsv): every complaint,
  whatever the channel, within one working day of receipt.
- [`registers/non-conforming-versions.tsv`](registers/non-conforming-versions.tsv):
  every finding that released versions do not conform.
- [`registers/upstream-reports.tsv`](registers/upstream-reports.tsv): every
  vulnerability reported to the maintainer of an integrated component
  ([below](#vulnerabilities-in-integrated-components)).

All three are public, appended and never rewritten. No row names a person or
carries patient data; the correspondence stays with Cadasto B.V., outside the
repository. A vulnerability enters both registers when its advisory is
published, so a row never discloses one before its fix. Until then the draft
advisory is the record.

## A complaint

1. Enter the row in the complaints register: `received`, `channel`,
   `versions`, a one-sentence `summary`, and `record`, the tracker issue or
   advisory that carries the work.
2. Classify it in `classification`:
   - `complaint`: FerroEHR behaves as documented and the report asks for
     something else;
   - `non-conformity`: a released version misses an EHDS Annex II item, an
     obligation of EHDS Chapter III or a CRA Annex I requirement; continue
     with [a non-conforming version](#a-non-conforming-version);
   - `serious-incident`: the event meets EHDS Art 2(2)(r); start
     [the serious-incident clock](#a-serious-incident-or-a-cra-art-14-notification) at once;
   - `exploited-vulnerability`: a vulnerability with reliable evidence of
     exploitation (CRA Art 3(42)), or a severe incident (CRA Art 14(5)); start
     [the CRA Art 14 clock](#a-serious-incident-or-a-cra-art-14-notification)
     at once.
3. Answer the person through the channel they used.
4. Close the row with `outcome` and `closed` when the record closes.

## A non-conforming version

EHDS Art 30(1)(i) asks for corrective action "without undue delay" once the
manufacturer considers, or has reason to believe, that a version does not
conform; CRA Art 13(21) asks for it "immediately".

1. Enter the row in the register of non-conforming versions: `found`,
   `versions` (each `X.Y.Z`, comma-separated), `requirement`, `summary`,
   `serious_incident`, `cra_notified`.
2. File the fix as a `Bug` at priority `Urgent` in the current milestone and
   ship it as the next patch release. A published release is immutable and
   there are no maintenance branches, so the fix ships forward
   (`.claude/rules/changelog.md`).
3. Decide between correction, withdrawal and recall, and write the decision
   and its timetable into `action`. Withdrawal is any measure preventing a
   product in the supply chain from being made available; recall is any
   measure achieving the return of a product already made available to the
   end user (Regulation (EU) 2019/1020 Art 3(22) and (23), which EHDS Art
   2(1)(d) and CRA Art 3(49) and (50) adopt). In this procedure a withdrawal
   is `scripts/release/withdraw.sh` plus the notices, and a recall adds a
   direct message to every known user to replace the version.
4. To withdraw, once the correcting release is published, run
   [`scripts/release/withdraw.sh`](#withdrawing-a-release).
5. Cadasto B.V. tells the national authorities (EHDS Art 30(1)(i)), with the
   notice text `withdraw.sh` prints. Fill `authorities_told` with the date and
   the Member States.
6. Cadasto B.V. tells distributors, the authorised representative, importers
   and users (EHDS Art 30(1)(j)): the advisory, the release notes of the
   correcting version, and a direct message where the user is known. Fill
   `users_told` with the date and the route.
7. Close the row with `corrected_in` and `closed`.

## A serious incident or a CRA Art 14 notification

The steps and deadlines are the FerroHEALTH procedure's. In this repository:

1. **On awareness.** Write down the date and hour. Enter the complaint row as
   `serious-incident` or `exploited-vulnerability`, and the non-conforming row
   with `serious_incident` set where it is one.
2. **The CRA notification.** Fill `cra_notified` with the date of each of the
   three steps as it is sent: the early warning, the notification and the
   final report.
3. **Inform users** of an actively exploited vulnerability or a severe
   incident through a GitHub security advisory on this repository and the
   release notes of the release that fixes it.
4. Continue as for [a non-conforming version](#a-non-conforming-version) from
   step 2.

## A fixed vulnerability's advisory

Every fixed vulnerability in FerroEHR gets a GitHub security advisory on this
repository, published with the release that fixes it: one in FerroEHR's own
code, and one for a dependency or base-image fix that changes what a shipped
artefact (a binary, an image, the Helm chart, a published `openehr-*` crate)
contains or how it behaves. A dependency finding that an OpenVEX statement
under `security/vex/` shows does not affect the artefact gets no advisory.
What the advisory carries, and when its technical details may wait, is the
FerroHEALTH [security policy](https://ferrohealth.eu/docs/security.html).

1. **On the report or the finding:** open a draft advisory on this repository.
   It is the record until publication.
2. **Ship the fix** as a security-only patch release. If a functional change
   has to ride with it, write the reason under the release's `### Security`
   heading (`.claude/rules/changelog.md`, release procedure).
3. **The changelog entry** under `### Security` carries the advisory's `GHSA-`
   identifier.
4. **Publish the advisory** when the release is published, and request a CVE
   through it where the vulnerability warrants one.
5. **Enter the register rows** the vulnerability needs (a complaint row for a
   reported one, a non-conformity row where it is one), now that the advisory
   is public.

## Vulnerabilities in integrated components

A vulnerability the maintainer confirms in a component FerroEHR integrates is
reported upstream as the FerroHEALTH procedure says (CRA Art 13(6)). For a
Rust crate the channel is its `SECURITY.md` or private vulnerability
reporting on its repository, then a RustSec advisory (`rustsec/advisory-db`)
once it can be public, so that `cargo audit` and `cargo deny` users see it.
FerroEHR's own side is handled like any other vulnerability: a draft
advisory, a security-only release when FerroEHR is affected, or an OpenVEX
statement when it is not.

The record is a row in
[`registers/upstream-reports.tsv`](registers/upstream-reports.tsv) once the
upstream advisory is public, or once the maintainer has declined one and the
agreed disclosure date has passed. The row names the component, the channel,
the upstream reference, whether a fix was shared, FerroEHR's own record and
the outcome, and never a person.

The `upstream-report` label on this tracker is for defects in the openEHR
specifications, not for component vulnerabilities.

## Withdrawing a release

```sh
scripts/release/withdraw.sh 4.3.3 --to 4.3.4 --dry-run   # read, print, change nothing
scripts/release/withdraw.sh 4.3.3 --to 4.3.4             # move the tags
```

The script refuses a version no row of the register names, a correcting
release that is not a later published release, and one that is itself in the
register. It moves the `<major>.<minor>` and `latest` tags of
`ghcr.io/ferrohealth/ferroehr`, `ferroehr-viewer` and `ferroehr-postgres` that
still point at the withdrawn digest to the correcting release. The
`<version>` and `sha-<commit>` tags, the digests, the GitHub release and its
attestations stay, so a deployment pinned to any of them keeps running the
withdrawn version until it moves. A `<major>.<minor>` tag whose line has no
correcting release stays, and the script names it for a manual move. The
Helm chart version cannot be marked deprecated in an OCI registry; the script
names it (from `--chart` or the release notes) for the advisory and
`SECURITY.md`. It then prints the advisory text, the `SECURITY.md` line, the
notices to the authorities and to users, and the register columns to fill.
The release-procedure side is in `.claude/rules/changelog.md` under
"Withdrawing a release".

## Facts only the owner supplies

These are open, and nothing in this repository answers them:

1. The CSIRT designated as coordinator for the Netherlands under CRA Art
   14(7), and Cadasto B.V.'s account on ENISA's single reporting platform
   (Art 16(1)).
2. Who at Cadasto B.V. decides that a vulnerability is actively exploited or
   an incident severe, and signs the CRA notification and the EHDS
   serious-incident report; and who stands in when that person is away, since
   the clock runs in hours.
3. The market surveillance authority of each Member State where FerroEHR is
   placed on the market or put into service, with its contact for a
   serious-incident report. EHDS Art 43(2) has the Commission and the Member
   States publish them.
4. Whether Cadasto B.V. is itself an essential or important entity under
   NIS2 in the Netherlands, in particular once it hosts FerroEHR as a service
   for other organisations (for example as a managed service provider or a
   cloud computing service provider).
5. The list of distributors, importers and known users to whom the
   Art 30(1)(j) and (n) notices go, which is the EHDS Art 35 register.
6. Whether `info@cadasto.com` should publish an encryption key for
   vulnerability reports, and who reads that inbox and forwards a report the
   same day, including outside working hours.
7. The 30-day ceiling on delaying an advisory's technical details, which
   `SECURITY.md` states, is the maintainers' proposal and needs Cadasto B.V.'s
   confirmation.
8. Who on the board of Cadasto B.V. decides a cessation of operations, and
   whether a successor would take over the support periods.
9. Whether the certified scope of Cadasto B.V.'s ISO 9001, ISO/IEC 27001 and
   NEN 7510 certificates covers the development and release of FerroEHR, the
   procedures in this file, and a FerroEHR service Cadasto B.V. hosts for
   other organisations. The certificates cover Cadasto B.V.'s management
   system within that scope; they never make FerroEHR a certified product.
