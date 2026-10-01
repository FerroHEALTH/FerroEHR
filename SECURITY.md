# Security Policy

FerroEHR is a clinical data repository; security reports are taken seriously
and handled with priority.

## Supported versions

**Only the most recent release is supported.** There are no maintenance
branches, no long-term-support line, and no backports. This is a deliberate
policy for the current release cadence, stated here because an operator
running a clinical deployment under change control has to plan around it.

| Artifact | Version line | What is supported |
|---|---|---|
| The server (GitHub releases, container images) | product SemVer, currently `4.x` | the newest `vX.Y.Z` tag, and nothing older |
| The Helm chart (`oci://ghcr.io/ferrohealth/charts`) | its own SemVer, independent of the server | the newest published chart version |
| The `openehr-*` crates (crates.io) | their own lockstep `0.0.x` line | the newest published version of all nine |

**How a security fix reaches you.** The fix lands on `main` and ships in the
next tagged release. That is normally the next *patch* on the current minor
(so a fix does not oblige you to take new behaviour), but the project does
not promise it: if the fix is only correct alongside a behavioural change, the
release carrying it is the release carrying the change, and the changelog
entry says so. A chart-only fix ships as a new chart version through the same
publish lane between server releases.

**When a version stops receiving fixes: the moment a newer release exists.**
A release is never retro-fitted: GitHub release immutability means a
published release's assets and tag cannot be modified at all, so the only
remedy for any defect is a new version. If you are not on the newest release,
you are receiving no security fixes, and the action is to upgrade.

**If you need something stronger than that** (a supported line you can stay
on, a backport window, a deprecation notice period), it does not exist today
and cannot be conjured by a policy document. Say so on the tracker; a
maintenance line is a commitment that needs people behind it, and the
[governance and continuity](GOVERNANCE.md) position records honestly how many
there currently are.

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
