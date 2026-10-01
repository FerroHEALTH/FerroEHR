# Governance

How decisions get made in FerroEHR, who makes them, and how that changes.

This document describes the project as it actually operates. Where the honest
description is "one person decides", it says so. A governance document that
describes a committee that does not meet is worse than none, because it
invites a reviewer to rely on a control that is not there.

## Current structure: benevolent dictator, one maintainer

FerroEHR has a single maintainer ([MAINTAINERS.md](MAINTAINERS.md)) who holds
final say on every decision: what gets built, what gets merged, what gets
released, and what the project refuses to do. There is no steering committee,
no technical oversight body, no foundation, and no vote.

This is the standard structure for a project of this age and size, and it has
the standard trade-off: decisions are fast and coherent, and the project's
resilience is one person's. The second half of that sentence is treated as a
finding rather than a footnote; see
[MAINTAINERS.md § If the maintainer is unavailable](MAINTAINERS.md#if-the-maintainer-is-unavailable).

## Where decisions are recorded

The tracker is the record. There is no separate design-document layer, and
that is deliberate: an architecture decision record that outlives the code it
justified becomes a false authority, so this project keeps decisions in the
places that cannot drift out of sync with the tree.

| Kind of decision                   | Where it lives                                                                                                   |
|------------------------------------|------------------------------------------------------------------------------------------------------------------|
| What to work on next               | a [GitHub issue](https://github.com/FerroHEALTH/FerroEHR/issues); the open list is the worklist                 |
| Direction and status, publicly     | the [FerroEHR Roadmap project board](https://github.com/FerroHEALTH/FerroEHR/projects), a view over the tracker |
| Why a change looks the way it does | the pull request description that landed it, and the issue's closing comment                                     |
| What a release contains            | [`CHANGELOG.md`](CHANGELOG.md) and the `vX.Y.Z` milestone                                                        |
| Standing architectural rules       | [`docs/architecture.md`](docs/architecture.md) and the `CLAUDE.md` files                                         |
| What conformance means             | the committed artifacts under `docs/conformance/`, produced by the CNF runner                                    |

A decision that exists only in a conversation is not a decision this project
made.

## How a change gets in

1. **An issue carries the contract**: what is wrong or missing, and the
   acceptance criteria that settle it.
2. **A pull request implements it** on a conventional-type branch, declaring
   `Closes #N`.
3. **The gates run.** They are not advisory and there is no override: format,
   clippy at `-D warnings`, the full test suite, the codegen drift check, the
   licence and advisory gates, the workflow security audit, the comment and
   documentation-claim guards, the Helm golden renders, and the browser
   battery. The complete list is [`.github/workflows/ci.yml`](.github/workflows/ci.yml);
   the merge gate is the single `conclusion` check, and a CI job that does not
   feed it fails its own guard.
4. **The maintainer merges.** A pull request from an account without write
   access additionally requires a code-owner approval before it can merge
   ([`.github/CODEOWNERS`](.github/CODEOWNERS)).

**On required review, stated plainly.** The maintainer's own changes are not
independently reviewed by a second human, because there is no second human.
Requiring two approvals of oneself would be a control that reports "reviewed"
without anyone having reviewed, and this project would rather report the truth
and let a deployer weigh it. What stands in for review here is machine
enforcement: the gates above, deterministic analysis on every pull request
(SonarQube Cloud and CodeQL) whose findings are advisory and never outrank
the specifications or these rules, and the conformance suite as an external
acceptance instrument.

## The specifications are the authority, not the maintainer

The one place the maintainer explicitly does *not* have final say is
specification conformance. The vendored openEHR specification text under
`docs/specs/openehr/` is the oracle for every wire-visible behaviour, and a
conformance failure is attributed against it — never against what the
implementation happens to do, and never against another CDR's behaviour. Where
the specifications are silent, the decision is the project's own and must be
labelled as such wherever it is written down.

If you believe the implementation contradicts the specification, that is not a
matter of taste and it is not the maintainer's call: cite the section and open
an issue. Those are the reports this project most wants.

## Becoming a maintainer

The route is open and it is the ordinary one:

1. **Contribute.** Sustained, merged, self-directed work. The bar is the
   point at which review stops finding things, not a pull-request count.
2. **Show judgement in the areas that matter here.** The signal is
   specification discipline: reading the normative text first-hand, citing it,
   refusing to resolve a question from another implementation's behaviour, and
   being honest when the specification is silent.
3. **Ask, or be asked.** Either direction is normal. Open an issue, or say so
   on a pull request.

The maintainer decides, and says yes or no with a reason on the tracker rather
than by silence. A new maintainer receives write access, a row in
[MAINTAINERS.md](MAINTAINERS.md), and their handle in
[`.github/CODEOWNERS`](.github/CODEOWNERS) for the areas they own. Publishing
identities move separately and only where the identity permits a second holder;
that table is in MAINTAINERS.md and is kept truthful.

## What this project will not do

Recorded here so the questions do not have to be re-litigated in each pull
request:

- **No copyright assignment.** You keep your copyright. Contributions are
  licensed under the project's Business Source License 1.1 for everyone
  including the maintainer (Apache License 2.0 for the `openehr-*` spec
  crates), become Apache 2.0 on each version's Change Date,
  and carry the relicensing grant in CONTRIBUTING.md § Licensing of
  contributions so the work stays one work under one licensor. This is a
  deliberate position, not an oversight.
- **No re-modelling of the openEHR specification by hand.** The specification
  crates are generated from the machine-readable artifacts; a change goes into
  the generator.
- **No weakening a test, a gate or an expectation to make a build green.** A
  red gate is information.
- **No claim the project cannot demonstrate.** Conformance numbers come from
  committed run artifacts, performance numbers from measured runs, and
  security controls from configuration a reader can check. If a claim has no
  evidence behind it, it does not get written.

## Code of conduct

[`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) applies to every space this project
occupies. Enforcement is the maintainer's, at the contact address given there.

## Changing this document

Governance changes are pull requests against this file, like anything else, and
they take effect when they merge. If the structure described here stops being
true (a second maintainer joins, a legal entity forms, a decision body is
created), this file changes in the same pull request that makes it true, not
afterwards.
