<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# FOSSA finding adjudications

Every other licence and advisory decision this project makes lives in git and is
reviewable: `deny.toml` carries dated reasons, `security/vex/*.openvex.json`
carries them again in a form a scanner consumes, and each vendored tree's
`PROVENANCE.md` records its terms and any election.

FOSSA is the exception, and not by our choice. Its licence policy rules and its
per-finding resolutions are **server-side only** — the CLI configuration file
cannot express either. The upstream documentation is explicit: `.fossa.yml`
*"does not define or create license policy rules"*, and it offers no field for
resolving or ignoring a finding. `project.policy` and `project.policyId` merely
reference a policy defined elsewhere, and both are create-only.

So without this file, the reasoning behind twenty dismissals would exist only in
a dashboard that no reader of this repository can see. This is that reasoning.

**This file is a record, not an enforcement.** What FOSSA actually acts on is
its own stored state. If the two disagree, the dashboard is what runs and this
file is what is wrong — check it before trusting it.

## The policy this is judged against

The project is attached to a FOSSA **licensing policy** chosen from a stock
template. The choice matters more than it looks: the templates differ on exactly
the licences whose obligations depend on how the artifact is linked.

FerroEHR ships compiled binary releases, and Rust links statically — a release
tarball is one `ferroehr` binary with every dependency inside it, with no
dynamic-linking boundary anywhere in the artifact. `GPL-2.0-only` is *flagged*
under the "Standard Bundle Distribution" template and *denied* under
"Single-Binary Distribution", and static linking is precisely the condition
under which that licence stops being a note and becomes an obligation.

## How to read the verdicts

`isLicenseDeclared: false` on a FOSSA finding means the licence was **inferred
by scanning file contents**, not read from the package's declared metadata.
Eighteen of the twenty findings carry that flag, and it accounts for nearly all
of them: a crate that vendors third-party C sources, or quotes a licence in a
test fixture, gets attributed a licence it does not declare.

Each verdict below therefore cites what the crate **declares** on crates.io.
That is checkable by anyone, without a FOSSA login:

```console
$ curl -s -H 'User-Agent: <you>' https://crates.io/api/v1/crates/aws-lc-sys \
    | jq -r '.versions[0].license'
ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND …
```

## False positives — the licence is not one the crate declares

| Crate | FOSSA inferred | crates.io declares | Why it fired |
|---|---|---|---|
| `aws-lc-sys 0.44.0` | GPL-1.0-only, GPL-1.0-or-later | `ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND …` | vendored AWS-LC C sources |
| `security-framework 3.7.0` | APSL-2.0 | `MIT OR Apache-2.0` | Apple header references |
| `fhir-model 0.13.0` | SimPL-2.0 | `MIT` | text-pattern match |
| `unsafe-libyaml-norway 0.2.15` | EUPL-1.2 | `MIT` | text-pattern match |
| `zstd-sys 2.0.16` | GPL-2.0-only | `MIT/Apache-2.0` | the vendored C zstd is BSD-3-Clause **OR** GPL-2.0; we take BSD-3-Clause |
| `pgp 0.20.0` | GPL-3.0-or-later | `MIT OR Apache-2.0` | text-pattern match |
| `ring 0.17.14` | openssl-ssleay | `Apache-2.0 AND ISC` | BoringSSL-derived sources; permissive either way |

All four findings FOSSA marks **denied** are in this table. None of them is a
licence this project may not ship.

The `pgp` entry was verified with particular care rather than waved through:
that crate produces the openPGP signatures on released versions, so a real
copyleft obligation there would matter more than most.

## Policy configuration — permissive licences the stock template flags

`Apache-2.0 WITH LLVM-exception` is the standard Rust and LLVM licence, and it
is strictly **more** permissive than plain Apache-2.0 — the exception removes
the binary attribution requirement. Flagging it while approving Apache-2.0 is
backwards, and both stock distribution templates do it.

`linux-raw-sys`, `rustix`, `symbolic-demangle`, `wasi`, `wasip2`,
`wit-bindgen` — plus `symbolic-demangle` again under
`apache-2.0-runtime-library-exception`.

The clean fix would be to approve those two licence forms in the policy, so the
rule stops firing at all. **That is not available on this plan**: custom
policies are a paid feature, and the stock templates cannot be edited. Both
distribution templates flag these forms, so switching between them does not help
either — verified by switching, after which the count did not move.

They are therefore dismissed per finding, which treats the symptom and leaves
the rule in place. Expect them to reappear when any of those crates changes
version, because the resolution attaches to the finding rather than to the
policy that produced it. That recurrence is a property of the tool on this
plan, not a sign anything regressed.

## Real, and accepted with a reason

| Crate | Licence | Adjudication |
|---|---|---|
| `r-efi 5.3.0`, `6.0.0` | `MIT OR Apache-2.0 OR LGPL-2.1-or-later` | **We elect MIT.** The licence is disjunctive, so no LGPL obligation attaches — the same form of election recorded for the ISO 13606 reference models under MPL-1.1 in `crates/openehr-adl/tests/corpus/rm/PROVENANCE.md`. |
| `leptos-chartistry 0.2.3` | `MPL-2.0` | Genuinely MPL, and the only finding whose licence FOSSA read from the declared metadata. MPL-2.0 is weak copyleft at **file** level: modifications to its own files stay MPL, and distributing it alongside MIT code is permitted. Reached only by the viewer. |

## Already adjudicated elsewhere

`rsa 0.9.10` — CVE-2023-49092, the Marvin timing side-channel (RUSTSEC-2023-0071).

Accepted in `deny.toml` with a dated reason and published as a machine-readable
statement in `security/vex/rust-advisories.openvex.json`. The justification is
that this project only *verifies* RS256 signatures, which is a public-key
operation, and never holds an RSA private key — so the private-key timing
oracle does not apply.

FOSSA does not consume OpenVEX, so the same reasoning has to be entered into it
by hand. That duplication is the cost of the tool, not a second decision.

## Maintenance, not licensing

`ordered-float 2.10.1` is reported outdated. The workspace pins `4.x`; this is
an older copy arriving transitively. Trace it with
`cargo tree -i ordered-float@2.10.1`.

## Why the licence gate is not switched on

`.github/workflows/fossa.yml` sets `run-tests: false`, so the lane analyses and
publishes but does not fail the build.

Of the original twenty findings, ten were false positives and seven are
permissive licences the stock policy flags in error. Gating on that would fail
the build for reasons no author could act on, and a permanently red gate is one
people learn to merge past — which costs more than the gate was ever worth.

The plan does not offer custom policies, and that is the deciding fact rather
than a detail. A gate is only worth having if the rule behind it can be
corrected when it is wrong; here it cannot. Every recurrence of the seven
permissive findings would have to be dismissed by hand, and each dismissal is a
judgement call made under pressure to get a build green — which is exactly the
condition under which a real finding gets waved through with the noise.

So `run-tests` stays `false`, and this is a settled position rather than a
deferral: the lane analyses, publishes and is read, and `cargo deny` remains
the gating advisory check, because its rules live in this repository where they
can be fixed.
