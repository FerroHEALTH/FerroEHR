# Post-market procedures

The working checklist Cadasto B.V., the manufacturer of each tagged FerroEHR
release, follows for a complaint, a non-conforming version, a serious
incident, an actively exploited vulnerability or severe incident, a fixed
vulnerability's advisory, a vulnerability in an integrated component, a
request from an authority, and the cessation of its operations. The duties and the deadlines come from Regulation
(EU) 2025/327, the EHDS (`docs/law/eu/ehds/text.html`), and Regulation (EU)
2024/2847, the CRA (`docs/law/eu/cra/text.html`); the steps that carry them
out are our own design. The manufacturer position is recorded in
[`architecture.md`](architecture.md#regulatory-position-ehds-and-cra). The
public account, with the quotations, is the book's
[Complaints, incidents and vulnerabilities](https://ferroehr.eu/docs/latest/compliance/post-market.html)
page; this file is what each person does, in order.

When each duty applies:

| Duty | Applies from | Text |
|---|---|---|
| CRA Art 14 notification | 11 September 2026, for every release, including those published before 11 December 2027 | CRA Art 71(2), Art 69(3) |
| EHDS Art 30 (complaints, registers, corrective action), Art 43 to 45 (market surveillance, serious incidents) | 26 March 2027 | EHDS Art 105, which lists none of them among the later dates |
| CRA Art 13(6), 13(21), 13(23), Annex I Part II (upstream reporting, corrective measures, cessation, vulnerability handling) | 11 December 2027 | CRA Art 71(2) |

The procedure runs now for all of them, so that it is practised before the
EHDS and the rest of the CRA apply.

## Who does what

| Who | What |
|---|---|
| Cadasto B.V., the manufacturer | Receives complaints and incident reports at `info@cadasto.com`, the single point of contact (EHDS Art 30(1)(g)). Decides whether an event is a serious incident or an actively exploited vulnerability, and signs every report to an authority. The person who decides is an owner fact (see the last section). |
| The maintainer, Ruben Talstra | Reads private vulnerability reports, the vulnerability reports Cadasto B.V. forwards from `info@cadasto.com`, and the GitHub tracker, enters the register rows, prepares the correcting release, runs `scripts/release/withdraw.sh`, drafts the advisory and the reports for Cadasto B.V. to send. |
| The deploying organisation | Supplies the facts of its deployment and the JSON document `ferroehr report` writes. Owes its own notifications under the GDPR and, where it is an essential or important entity, under NIS2. None of the manufacturer's reports stands in for them. |

## The channels

| What | Channel | Who reads it |
|---|---|---|
| A vulnerability, including one seen exploited | GitHub private vulnerability reporting ([`SECURITY.md`](../SECURITY.md)) | the maintainer, the same working day where possible |
| A vulnerability, by email (CRA Art 13(17): the single point of contact "shall not limit such means to automated tools") | `info@cadasto.com`, subject "FerroEHR vulnerability" | Cadasto B.V., forwarded to the maintainer the same day |
| A possible serious incident: anything that harmed a person or could | `info@cadasto.com`, subject "FerroEHR incident" | Cadasto B.V., forwarded to the maintainer the same day |
| Any other complaint | `info@cadasto.com`, subject "FerroEHR complaint", or a public GitHub issue | Cadasto B.V. and the maintainer |

An emailed vulnerability report runs through exactly the procedure a private
GitHub report does. On receipt, the maintainer opens a draft security advisory
on the repository for it (adding the reporter as a collaborator on the draft
only if they have a GitHub account and want it; otherwise the conversation
stays on email), and the acknowledgement and assessment times of
`SECURITY.md` run from the email's arrival. The email's arrival is also the
moment the manufacturer becomes aware for CRA Art 14: if the report says the
vulnerability is exploited, the 24-hour clock of
[the CRA Art 14 procedure](#an-actively-exploited-vulnerability-or-severe-incident-cra-art-14)
starts then, not when the draft advisory is opened. Whoever reads
`info@cadasto.com` therefore forwards a message with "vulnerability" or
"exploited" in it to the maintainer at once, and writes down the hour it
arrived.

Distributors, importers and the authorised representative, where there are
any, are told of these channels and of the registers when they are supplied,
and of each new register row (EHDS Art 30(1)(n) and (o)).

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
     [the serious-incident clock](#a-serious-incident-ehds-art-447) at once;
   - `exploited-vulnerability`: a vulnerability with reliable evidence of
     exploitation (CRA Art 3(42)), or a severe incident (CRA Art 14(5)); start
     [the CRA Art 14 clock](#an-actively-exploited-vulnerability-or-severe-incident-cra-art-14)
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
5. Tell the national authorities of each Member State where the version was
   made available or put into service: the non-conformity, the corrective
   action with its timetable, and the date the version was brought into
   conformity, recalled or withdrawn (EHDS Art 30(1)(i)). Fill
   `authorities_told` with the date and the Member States.
6. Tell distributors, the authorised representative, importers and users
   (EHDS Art 30(1)(j)): the advisory, the release notes of the correcting
   version, and a direct message where the user is known. Fill `users_told`
   with the date and the route.
7. Close the row with `corrected_in` and `closed`.

## A serious incident (EHDS Art 44(7))

A serious incident is "any malfunction or deterioration in the
characteristics or performance of an EHR system made available on the market
that directly or indirectly leads, might have led or might lead to" a death
or serious harm to health, serious prejudice to a person's rights, or serious
disruption of critical health infrastructure (EHDS Art 2(2)(r)). For a
clinical data repository the plausible ones include:

- a committed version lost, or attributed to the wrong EHR;
- a read or an AQL result that returns another patient's data;
- data under a restriction of processing (#3324) served on a path that should
  refuse it;
- an access the logging component did not record;
- a signature verification that accepts a tampered version.

The report is due "immediately after the manufacturer has established a
causal link ... or the reasonable likelihood of such a link and, in any
event, not later than three days after the manufacturer becomes aware".

1. **Day 0, on awareness.** Write down the date and hour. Enter the complaint
   row as `serious-incident` and the non-conforming row with
   `serious_incident` set. Ask the deployment how it is deployed, what
   happened, and for the JSON document `ferroehr report` writes, which
   identifies the version and build.
2. **The same day.** Check whether the event is also a CRA Art 14 matter (an
   actively exploited vulnerability, or a severe incident affecting the
   security of FerroEHR). If it is, run that clock as well; neither report
   replaces the other.
3. **By day 3 at the latest.** Cadasto B.V. reports to the market
   surveillance authority of each Member State where the incident occurred
   and of each Member State where FerroEHR is placed on the market or put
   into service: what happened, the versions involved, the causal link as far
   as it is established, and "a description of the corrective action taken or
   envisaged". A report sent before the cause is known says so, and is
   completed when it is.
4. **NIS2, separately.** The report is "without prejudice to" NIS2. Tell the
   deploying organisation the same day what it needs for its own NIS2
   notification (see [NIS2](#nis2-kept-separate)).
5. **Harm.** Where an authority finds that FerroEHR caused harm to health or
   safety, give the affected person or user the information and documentation
   at once (EHDS Art 44(3)), within data protection rules.
6. Continue as for [a non-conforming version](#a-non-conforming-version) from
   step 2, for every copy placed on the market in the Union (Art 44(4)).

## An actively exploited vulnerability or severe incident (CRA Art 14)

The manufacturer "becomes aware" through a private vulnerability report, an
emailed one, a CSIRT telling it of someone else's notification (CRA Art 15(4)), or its own
finding. A report through private vulnerability reporting that says the
vulnerability is exploited starts the clock.

1. **Hour 0.** Write down the date and hour. Cadasto B.V. decides, on the
   evidence, whether the vulnerability is actively exploited ("reliable
   evidence that a malicious actor has exploited it in a system without
   permission of the system owner", CRA Art 3(42)) or the incident is severe
   (CRA Art 14(5): it affects or can affect the ability to protect the
   availability, authenticity, integrity or confidentiality of sensitive or
   important data or functions, or it led or can lead to malicious code).
2. **Within 24 hours: the early warning** (Art 14(2)(a), for an incident Art
   14(4)(a)), through ENISA's single reporting platform to the CSIRT
   designated as coordinator in the Netherlands, visible to ENISA at the same
   time (Art 14(1), (3) and (7)): that it happened, and the Member States
   where Cadasto B.V. knows the affected release is made available; for an
   incident, whether it is suspected to be unlawful or malicious. Fill the
   first date in `cra_notified`.
3. **Within 72 hours: the notification** (Art 14(2)(b), 14(4)(b)): the
   release concerned, the nature of the exploit or the incident, the
   corrective or mitigating measures taken and those users can take, and how
   sensitive Cadasto B.V. considers the information. Where a fix is expected
   within 72 hours, say so: that is the condition under which the receiving
   CSIRT may delay passing the notification on (Commission Delegated
   Regulation (EU) 2026/881 Art 3(a), `docs/law/eu/cra-dissemination-delay-2026-881/text.html`,
   adopted under CRA Art 14(9) for the delay CRA Art 16(2) allows). Fill the
   second date.
4. **Inform users** (Art 14(8)): a GitHub security advisory on this
   repository and the release notes of the release that fixes it, with the
   mitigations users can apply before they upgrade. The advisory is
   machine-readable through the GitHub REST API.
5. **On request: an intermediate report** to the CSIRT (Art 14(6)).
6. **The final report**: for a vulnerability, no later than 14 days after a
   corrective or mitigating measure is available (Art 14(2)(c)); for an
   incident, within one month after the notification (Art 14(4)(c)). Fill
   the third date.
7. Continue as for [a non-conforming version](#a-non-conforming-version). If
   the event is also an EHDS serious incident, the three-day report above is
   due as well.

## NIS2, kept separate

NIS2 (`docs/law/eu/nis2/text.html`) is a Directive: its Art 23 has each
Member State require essential and important entities to notify significant
incidents to their CSIRT or competent authority (an early warning within 24
hours, an incident notification within 72 hours, a final report within one
month, Art 23(4)). Healthcare providers are a sector of Annex I, point 5. A
hospital that runs FerroEHR may owe that notification under its own Member
State's law; neither Cadasto B.V.'s EHDS report nor its CRA notification
discharges it, and the hospital's notification does not discharge
Cadasto B.V.'s. What Cadasto B.V. gives the hospital is the facts: the
advisory, the affected versions, the mitigations, and the timeline it
reported.

## A fixed vulnerability's advisory (CRA Annex I Part II(4) and (8))

Every fixed vulnerability gets a GitHub security advisory, published with the
release that fixes it (`SECURITY.md` §"Security advisories"): one in
FerroEHR's own code, and one for a dependency or base-image fix that changes
what a shipped artefact (a binary, an image, the Helm chart, a published
`openehr-*` crate) contains or how it behaves. A dependency finding that an
OpenVEX statement under `security/vex/` shows does not affect the artefact
gets no advisory.

1. **On the report or the finding:** open a draft advisory on the repository.
   It is the record until publication. Assess the severity as a CVSS vector
   and score, and name the affected versions of each artefact.
2. **Decide on a delay** (Part II(4), "in duly justified cases"). Cadasto B.V.
   delays only the technical description and any reproduction, and only when
   all three criteria of `SECURITY.md` hold: exploitable without credentials
   or by any authenticated caller against an unpatched deployment, no
   mitigation short of upgrading, and the details not public and the
   vulnerability not actively exploited. Write the decision and the reason
   into the draft. The delay ends no later than 30 days after the fixing
   release, or when the details become public elsewhere.
3. **Ship the fix** as a security-only patch release. If a functional change
   has to ride with it, write the reason under the release's `### Security`
   heading (`.claude/rules/changelog.md`, release procedure).
4. **The changelog entry** under `### Security` carries the advisory's `GHSA-`
   identifier.
5. **Publish the advisory** when the release is published: description,
   impact, CVSS, affected and fixed versions, remediation (the release to
   upgrade to, and any mitigation that works before), the CVE when assigned,
   and credit. Request a CVE through the advisory where the vulnerability
   warrants one.
6. **Enter the register rows** the vulnerability needs (a complaint row for a
   reported one, a non-conformity row where it is one), now that the advisory
   is public.

## Vulnerabilities in integrated components (CRA Art 13(6))

CRA Art 13(6): a manufacturer that identifies a vulnerability "in a component,
including in an open source-component, which is integrated in the product"
shall "report the vulnerability to the person or entity manufacturing or
maintaining the component", and where it has written a fix, "share the
relevant code or documentation" with them, "where appropriate in a
machine-readable format".

- **Who reports:** the maintainer, on behalf of Cadasto B.V.
- **When:** as soon as the vulnerability is confirmed in the component, whether
  FerroEHR found it (a fuzz crash in a dependency, a review, a test) or a
  reporter told us of it. A component vulnerability a scanner reports from a
  published advisory is already known upstream and needs no report.
- **Through which channel:** the component's own security policy first. For a
  Rust crate, its `SECURITY.md` or private vulnerability reporting on its
  repository, then a RustSec advisory (`rustsec/advisory-db`) once it can be
  public, so that `cargo audit` and `cargo deny` users see it; for a base-image package (PostgreSQL, Debian in the distroless
  layer), the project's security contact or the distribution's security
  tracker; for a vendored asset, its publisher's contact.
- **The fix:** a patch FerroEHR wrote is offered upstream as a pull request or
  a patch file against the component's repository, under the component's
  licence.
- **FerroEHR's own side:** the vulnerability is handled under Annex I Part II
  like any other: a draft advisory, a security-only release when FerroEHR is
  affected, or an OpenVEX statement when it is not.
- **The record:** a row in
  [`registers/upstream-reports.tsv`](registers/upstream-reports.tsv) once the
  upstream advisory is public, or once the maintainer has declined one and
  the agreed disclosure date has passed. The row names the component, the
  channel, the upstream reference, whether a fix was shared, FerroEHR's own
  record and the outcome, and never a person. Until then the upstream report
  is the record.

The `upstream-report` label on this tracker is for defects in the openEHR
specifications, not for component vulnerabilities.

## Cessation of operations (CRA Art 13(23))

CRA Art 13(23): "A manufacturer that ceases its operations and, as a result, is
not able to comply with this Regulation shall inform, before the cessation of
operations takes effect, the relevant market surveillance authorities as well
as, by any means available and to the extent possible, the users of the
relevant products with digital elements placed on the market, of the impending
cessation of operations."

1. **Who decides:** the board of Cadasto B.V. decides that it will cease
   operations, or that FerroEHR will no longer be maintained, and the date it
   takes effect. The steps below run as soon as that date is set, so that
   every notice goes out before it.
2. **The authorities first:** the market surveillance authority of each Member
   State where FerroEHR is made available, and, while the EHDS applies, the
   authority of each Member State where it is put into service. The notice
   names the products (the server, the images, the chart, the crates), the
   date, the date from which vulnerabilities are no longer handled, and what
   stays published.
3. **The known users:** a direct message to every operator in the EHDS Art 35
   register of economic operators supplied, and to every commercial licensee.
4. **The public notice:** a pinned GitHub issue on the repository, a notice at
   the top of `SECURITY.md` and on the documentation site's landing page, and
   a final security advisory naming the last supported release.
5. **What stays:** every release, image, chart version, crate, advisory, the
   registers and the documentation stay published and are not withdrawn.
   `SECURITY.md` states that the support periods end on the cessation date.
6. **Last register rows:** close every open row of the three registers with
   the outcome as it stands.

## A request from an authority

A market surveillance authority's request for information or documentation is
answered in an official language of the Member State concerned (EHDS Art
30(1)(l)), on paper or electronically, in a language the authority easily
understands (Art 30(5); CRA Art 13(22)). Cadasto B.V. cooperates on any
action to bring FerroEHR into conformity or to eliminate its risks (EHDS Art
30(1)(m), Art 30(5), Art 44(1)). The technical documentation and the
declaration of conformity an answer draws on are #3616. Keep the request and
the answer, outside the repository. An authority may restrict, recall or
withdraw FerroEHR when the manufacturer does not cooperate or answers
incompletely or incorrectly (EHDS Art 43(5)).

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
