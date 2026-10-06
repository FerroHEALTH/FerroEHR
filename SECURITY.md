# Security Policy

FerroEHR is a clinical data repository; security reports are taken seriously
and handled with priority.

## Supported versions

Cadasto B.V. is the manufacturer of each tagged FerroEHR release under the
Cyber Resilience Act, Regulation (EU) 2024/2847 (the position is recorded in
[`docs/architecture.md`](docs/architecture.md#regulatory-position-ehds-and-cra)).

**The support period of a release is five years from the month it is
published.** CRA Art 13(8) sets five years as the minimum where a product is
expected to be in use for longer, and a clinical data repository is. The end
date (month and year, Art 13(19) and Annex II point 7) is printed in each
release's notes and in the table below. During that period vulnerabilities in
the release are handled under Annex I Part II.

**The security update for a release in its support period ships in the newest
release.** There are no maintenance branches, no long-term-support line and
no backports. CRA Art 13(10) allows a manufacturer of software placed on the
market in successive substantially modified versions to handle vulnerabilities
in the version it placed last, provided the users of earlier versions have
access to that version free of charge and without extra cost to adjust their
environment. Every FerroEHR release is published under the same licence terms
as the one before it, so a user with the right to run one release has the same
right to run the newest. Whether this holds for every commercial licence is a
point for counsel listed in `docs/architecture.md`; a commercial licensee for
whom it does not hold should raise it with Cadasto B.V. at
[info@cadasto.com](mailto:info@cadasto.com).

| Artifact | Version line | Support period | Where the update ships |
|---|---|---|---|
| The server (GitHub releases, container images) | product SemVer, currently `4.x` | five years from the month of each `vX.Y.Z` tag; the end date is in the release notes | the newest `vX.Y.Z` tag |
| The Helm chart (`oci://ghcr.io/ferrohealth/charts`) | its own SemVer, independent of the server | five years from the month the chart version is published | the newest published chart version |
| The `openehr-*` crates (crates.io) | their own lockstep `0.0.x` line | five years from the month of publication | the newest published version of all nine |

Releases published before 2026-10-06 carry no end date in their notes; their
support period is five years from the month of their tag all the same.

**How a security fix reaches you.** The fix lands on `main` and ships in the
next tagged release. That is normally the next *patch* on the current minor
(so a fix does not oblige you to take new behaviour), but the project does
not promise it: if the fix is only correct alongside a behavioural change, the
release carrying it is the release carrying the change, and the changelog
entry says so. A chart-only fix ships as a new chart version through the same
publish lane between server releases. Each security update stays available
for at least ten years after it is issued, or for the rest of the support
period if that is longer (Art 13(9)): releases are immutable and are never
deleted.

**Running an older release.** A release is never retro-fitted: GitHub release
immutability means a published release's assets and tag cannot be modified,
so the only remedy for a defect is a new version. If you are not on the newest
release you are not receiving the fix, and the action is to upgrade. Old
releases stay downloadable as an archive (Art 13(11)); running one that a
newer release has fixed exposes you to the vulnerability the newer release
fixes, and its advisory says which.

**A withdrawn release is unsupported.** A release found not to conform is
entered in the register of non-conforming versions and withdrawn (the
[post-market procedures](docs/post-market.md)); this section then lists it.
No release has been withdrawn.

## Reporting a vulnerability

**Please do not open a public issue for suspected vulnerabilities.**

Report privately via
[GitHub private vulnerability reporting](https://github.com/FerroHEALTH/FerroEHR/security/advisories/new)
("Report a vulnerability" on the repository's Security tab).

Include what you can: affected component/endpoint, reproduction steps or a proof
of concept, impact assessment, and any suggested fix.

### What you can expect from us

- **An acknowledgement within 5 working days.** If you have not heard anything by
  then, the report has not reached us; escalate by opening a public issue
  saying only that a private report is awaiting acknowledgement, with no details.
- An assessment with a severity and an intended fix window within 10 working
  days of the acknowledgement.
- Coordinated disclosure: we will agree a date with you rather than impose one,
  and we will tell you when the fix ships.

These are commitments to you, not conditions on you. If we miss them, publishing
is your call.

### Safe harbour

We will not pursue or support legal action against anyone who reports a
vulnerability in good faith and follows this policy. In practice that
means: you tested against
your own deployment or a test instance you control, you did not access, modify or
retain data belonging to anyone else, you did not degrade service for others, and
you gave us the window above before publishing. If you are unsure whether
something is in scope, ask first; a question is always in good faith.

**Never test against a live clinical deployment you do not own.** This software
holds patient data.

### Credit

We name reporters in the advisory and the changelog by default, using whatever
name and link you give us. Tell us if you would rather not be named; declining
credit costs you nothing and changes nothing about how the report is handled.

### What Cadasto B.V. reports to the authorities (CRA Art 14)

Since 11 September 2026 the manufacturer notifies every actively exploited
vulnerability in FerroEHR, and every severe incident having an impact on its
security, that it becomes aware of (CRA Art 14(1) and (3), applying from that
date under Art 71(2) and reaching releases published before 11 December 2027
under Art 69(3)). The notification goes through ENISA's single reporting
platform to the CSIRT designated as coordinator in the Netherlands, where
Cadasto B.V. has its main establishment, and is visible to ENISA at the same
time (Art 14(7)). It comes in three steps:

| Step | Deadline, from the moment the manufacturer becomes aware | Content |
|---|---|---|
| Early warning | 24 hours | that it happened, and the Member States where the manufacturer knows the affected release is made available; for an incident, whether it is suspected to be unlawful or malicious (Art 14(2)(a), 14(4)(a)) |
| Notification | 72 hours | the release concerned, the nature of the exploit or incident, the corrective or mitigating measures taken and those users can take (Art 14(2)(b), 14(4)(b)) |
| Final report | a vulnerability: 14 days after a corrective or mitigating measure is available; an incident: one month after the notification | the description, severity and impact, what is known of the actor or the root cause, and the update or measures (Art 14(2)(c), 14(4)(c)) |

A report you send us through private vulnerability reporting is one way the
manufacturer becomes aware, so a report of a vulnerability you have seen
exploited starts the 24-hour clock. Say so in the report. Coordinated
disclosure with you continues alongside the notification; the CSIRT can delay
passing it on while a fix is prepared (Art 16(2)).

Users are told of the vulnerability or incident, and of what they can do about
it, through a GitHub security advisory on this repository and the release notes
of the release that fixes it (Art 14(8)). The advisories are machine-readable
through the GitHub REST API (`GET /repos/FerroHEALTH/FerroEHR/security-advisories`),
and an advisory that names a published crate also reaches the GitHub Advisory
Database in OSV format.

The steps the maintainers follow, including who decides that a vulnerability is
actively exploited, are in [`docs/post-market.md`](docs/post-market.md). An
event that is also an EHDS serious incident, or for which a deploying hospital
owes a NIS2 notification of its own, is reported each way it has to be; the
procedure says how the three relate.

## Scope notes

- The server handles PHI-class data by design; reports about data exposure
  through the API, AQL, telemetry, or the audit trail are in scope even when
  they look like "just configuration".
- Supply-chain policy is enforced in CI by `cargo deny`, which reads the same
  RustSec database `cargo audit` does and adds yanked/licence/source checks on
  top; known advisories that are deliberately accepted are documented with their
  rationale in [`deny.toml`](deny.toml), which is the single advisory gate.
- An advisory reported by a scanner that reads `Cargo.lock` (including the
  OpenSSF Scorecard's) may name a crate our feature set never compiles, because
  the lock file records every dependency any feature combination *could* pull.
  `deny.toml`'s header explains the asymmetry and carries the current example;
  such a report is not an accepted risk, it is a dependency that does not exist
  in the built artifact.
- Findings in an inherited upstream container layer that we have argued are not
  reachable are published as OpenVEX documents under
  [`security/vex/`](security/vex/), with the justification and an impact
  statement you can check. If you think one of those arguments is wrong, that is
  a valid report.

## Repository security settings — the posture of record

Settings live in GitHub, not in the tree, so they can be changed without a
commit and reset without anyone noticing. This table is the record of what the
posture is **supposed** to be; read it back with
`gh api repos/FerroHEALTH/FerroEHR --jq '.security_and_analysis'` and
`gh api repos/FerroHEALTH/FerroEHR/rulesets`, and treat a divergence as a
finding.

| Setting | Expected | Why |
|---|---|---|
| Secret scanning | enabled | the baseline detector |
| Push protection | enabled | refuses the commit rather than filing an alert after the fact |
| Secret scanning — **non-provider patterns** | enabled | the credential classes this repository is most likely to leak are not provider tokens: private keys, database URLs with an embedded password, HTTP basic-auth URLs, generic high-entropy secrets. The chart mounts a config volume carrying a private key, the OIDC configuration takes an HMAC secret with an enforced entropy floor, and the platform library ships a signing module |
| Secret scanning — **validity checks** | enabled | the difference between "rotate this eventually" and "this credential is live right now" |
| Dependabot security updates | enabled | advisory-driven bumps, exempt from the update cooldowns |
| Private vulnerability reporting | enabled | the reporting route this document points at |
| Ruleset `main` (default branch) | active — no deletion, no force-push, signed commits, pull request required (code-owner review, stale approvals dismissed on push), `conclusion` status check required on an up-to-date branch; repository admins may bypass (the merge-on-local-gates lever — a deliberate, recorded trade against the Scorecard admin-enforcement warning) | the merge gate |
| Ruleset `release-tags` (`refs/tags/v*`) | active — no tag deletion, no non-fast-forward tag update, signatures required | three lanes publish off a raw tag push (the release, the Helm chart, the documentation version cut). Release immutability protects the window *after* a release is published; this protects the window in which a tag drives a build, an image push and a chart publish |

> [!NOTE]
> Two of these are **not yet true**: `secret_scanning_non_provider_patterns`
> and `secret_scanning_validity_checks` both read `disabled`. Both are
> accepted-and-ignored by `PATCH /repos/{owner}/{repo}` — the request returns
> `200` and changes nothing — so they have to be switched on in the repository
> Settings UI (*Settings → Code security → Secret Protection*). The table above
> states the intended posture; this note is what makes the gap visible rather
> than implied.

### Reporting a vulnerability in Kubernetes itself

A vulnerability in the Kubernetes platform — the API server, kubelet, etcd, a CNI
or a container runtime — is reported to Kubernetes, not here:
<https://kubernetes.io/docs/reference/issues-security/security/>. Advisories are
published on `kubernetes-announce` and the [official CVE
feed](https://kubernetes.io/docs/reference/issues-security/official-cve-feed/);
following them is the cluster operator's responsibility, as recorded in the
[cluster-hardening chapter](website/book/src/installation/kubernetes-hardening.md).
A vulnerability in FerroEHR — including in the Helm chart — comes to us through
the process above.

## Machine-readable policy

This policy is also published as
[`security.txt`](https://ferroehr.eu/.well-known/security.txt) per
[RFC 9116](https://www.rfc-editor.org/rfc/rfc9116).
