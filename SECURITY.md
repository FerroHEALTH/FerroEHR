# Security Policy

FerroEHR is a clinical data repository; security reports are taken seriously
and handled with priority.

Cadasto B.V. is the manufacturer of every FerroHEALTH product, FerroEHR
included, and handles vulnerabilities in all of them under one policy:
the response times, coordinated disclosure, the safe harbour, credit, what a
security advisory carries and when its details may wait, security-only patch
releases, what the manufacturer reports to the authorities under CRA Art 14,
and what happens if it ceases operations. That policy is published in the
FerroHEALTH book: <https://ferrohealth.eu/docs/security.html>. The
manufacturer's details are on <https://ferrohealth.eu/docs/manufacturer.html>.

This file keeps what is particular to FerroEHR: which versions are
supported, how to report a vulnerability in this repository's products, what
is in scope, and the repository's own security settings.

## Supported versions

Cadasto B.V. is the manufacturer of each tagged FerroEHR release under the
Cyber Resilience Act, Regulation (EU) 2024/2847 (the position is recorded in
[`docs/architecture.md`](docs/architecture.md#regulatory-position-ehds-and-cra)).

**Only the newest release is supported.** When a new release is published,
every earlier release stops receiving fixes: there are no maintenance
branches, no long-term-support line and no backports. If you are not on the
newest release, upgrade; upgrading is free. Under the Cyber Resilience Act,
Cadasto B.V. commits to keep publishing security fixes for at least five years
after each release (Art 13(8)) and ships them only in the newest release
(Art 13(10)).

The five years are a commitment for the product as a whole. Art 13(8) sets five years as the minimum where a product is
expected to be in use for longer, and a clinical data repository is. Art 13(10)
lets the manufacturer deliver security updates only in the version last placed
on the market, provided users of earlier versions receive it free of charge
with no added cost to adjust their environment, which is how every FerroEHR
upgrade is delivered. The end month of the commitment for a release (month
and year, Art 13(19) and Annex II point 7) is printed in its release notes:
it is the date until which Cadasto B.V. keeps shipping fixed releases, and the
release itself is supported only while it is the newest. Vulnerabilities are
handled under Annex I Part II; the FerroHEALTH
[security policy](https://ferrohealth.eu/docs/security.html) states the same
policy for every FerroHEALTH product.

| Artifact | Supported version | Fixes ship in |
|---|---|---|
| The server (GitHub releases, container images) | the newest published `vX.Y.Z` tag | the newest `vX.Y.Z` tag |
| The Helm chart (`oci://ghcr.io/ferrohealth/charts`) | the newest published chart version | the newest published chart version |
| The `openehr-*` crates (crates.io) | the newest published version of all nine | the newest published version of all nine |

Releases published before 2026-10-06 carry no end month in their notes. The
same rule applies to them: they are supported only while they are the newest,
and a fix for a vulnerability in one ships in the newest release.

**The running server states the end month of the commitment.** It is on the
boot banner, in `ferroehr --version`, under `support` in
`GET /management/info` and in `GET /management/status` as `support.status`
(`supported`, `ended` or `unreleased`; `supported` means the commitment for
that release is still running, and says nothing about whether a newer release
exists). Once the end month has passed the server logs a warning at boot and once a day (Art 13(19)). It does not contact anyone
to look for a newer release. To hear of new releases and fixes, subscribe to
the release feed, `https://github.com/FerroHEALTH/FerroEHR/releases.atom`, and
poll the published advisories,
`GET https://api.github.com/repos/FerroHEALTH/FerroEHR/security-advisories`.

**Running an older release.** A published release's assets and tag cannot be
modified, so the only remedy for a defect is a new version. If you are not on
the newest release you are not receiving the fix, and the action is to
upgrade.

**A withdrawn release is unsupported.** A release found not to conform is
entered in the register of non-conforming versions and withdrawn (the
[post-market procedures](docs/post-market.md)); this section then lists it.
No release has been withdrawn.

## Reporting a vulnerability

**Please do not open a public issue for suspected vulnerabilities.**

This route covers the products this repository publishes: the FerroEHR server
and its container images (`ferroehr`, `ferroehr-viewer`, `ferroehr-postgres`),
the Helm chart, and the nine `openehr-*` crates on crates.io. Report
privately, through either route:

- [GitHub private vulnerability reporting](https://github.com/FerroHEALTH/FerroEHR/security/advisories/new)
  ("Report a vulnerability" on the repository's Security tab), which needs a
  GitHub account; or
- email to [info@cadasto.com](mailto:info@cadasto.com), the single point of
  contact of Cadasto B.V., the manufacturer (CRA Art 13(17)), with "FerroEHR
  vulnerability" in the subject. No account is needed. The address has no
  published encryption key, so send the details you are comfortable sending by
  email and say that more is available; we will agree a channel for the rest.

Both routes reach the same people and the same procedure. Include what you
can: affected component/endpoint, reproduction steps or a proof of concept,
impact assessment, and any suggested fix. If you have seen the vulnerability
exploited, say so.

FerroEHR is solo-maintained, so the response times are best effort rather
than a contractual service-level agreement. They are the times every product
of the Ferro family states: an acknowledgement within about 5 business days,
and an initial assessment (is it valid, how severe) within about 10 business
days of the report. A confirmed vulnerability is fixed in a release published
within the target for its severity, counted from the confirmed assessment:

| Severity | Fixed release published within |
|---|---|
| Critical | 7 days |
| High | 30 days |
| Medium | 90 days |
| Low | the next release |

The rest of what you can expect from us (coordinated disclosure, the safe
harbour and credit) is the FerroHEALTH
[security policy](https://ferrohealth.eu/docs/security.html).

**Never test against a live clinical deployment you do not own.** This software
holds patient data.

Fixed vulnerabilities in FerroEHR are published as GitHub security advisories
on this repository, with the fixing release. The `### Security` entry for the
fix in [`CHANGELOG.md`](CHANGELOG.md) carries the advisory's `GHSA-`
identifier, and the release notes repeat it. The advisories are
machine-readable through the GitHub REST API
(`GET /repos/FerroHEALTH/FerroEHR/security-advisories`), and an advisory that
names a published crate also reaches the GitHub Advisory Database in OSV
format.

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
- A vulnerability we find, or are told of, in a component FerroEHR integrates
  (a Rust crate, the PostgreSQL or distroless base image, a vendored asset) is
  reported to that component's maintainer through its own channel (CRA Art
  13(6)), and a fix we write for it is offered upstream. Each such report is
  entered in the public register
  [`docs/registers/upstream-reports.tsv`](docs/registers/upstream-reports.tsv)
  once the upstream advisory is out, and the steps are in
  [`docs/post-market.md`](docs/post-market.md#vulnerabilities-in-integrated-components).
  A vulnerability in a component that you found yourself goes to that
  component's maintainer; tell us as well if it reaches FerroEHR.

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
| Private vulnerability reporting | enabled | one of the two reporting routes this document points at |
| Ruleset `main` (default branch) | active: no deletion, no force-push, signed commits, pull request required (no approving review, by the owner's ruling of 2026-09-12; stale approvals dismissed on push), the `conclusion` status check required, and every merge through the merge queue (squash, all entries green). Organization admins may bypass, always. A bypass merge does not wait for `conclusion`, so it lands without the verdict of any lane still running, `ui-e2e` included (#4050, #4067). The bypass stays as the merge-on-local-gates lever and the way out when the queue or a lane is broken, a deliberate trade against the Scorecard admin-enforcement warning; a pull request whose lanes still run goes through the queue (`gh pr merge --auto`), never a bypass | the merge gate |
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
