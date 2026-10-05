# Why FerroEHR exists

This chapter is the project's position: what makes openEHR worth implementing,
what this implementation commits to, and what it offers the organisations that
run it and build on it. Read it if you are deciding whether to depend on FerroEHR, or
whether to contribute to it.

<!-- toc -->

## openEHR is worth building for

openEHR does something almost nothing else in health IT does: it separates
clinical knowledge from software, and then writes both down. What a blood
pressure, a medication order or a discharge summary *means* lives in
archetypes and templates authored by clinicians and modellers: published and
computable. The Reference Model underneath them is specified. So is
the query language, the REST interface, and the serialization, down to the
shape of the JSON on the wire.

The consequence is determinism. Given the same template and the same
composition, two conformant systems store the same record and answer the same
AQL query the same way. A clinical record stops being one application's
private state and becomes data that outlives the application, the vendor and
the procurement cycle. For a record that has to stay readable in twenty
years, by software nobody has written yet, that is the whole game.

## A specification is only as strong as the implementations you can run

A standard becomes real when there is something you can start with one
command, read the source of, check against the specification yourself, and
run for research, teaching or a non-profit deployment without a fee. Without
that, an excellent specification stays an idea that only well-funded
organisations can act on.

That is the gap this project set out to close: one complete, openly
developed, source-available openEHR CDR whose conformance is *measured
and published*.

## What we commit to

- **One licence for all of our own code, with no open-core tier.**
  Role- and attribute-based access control, IHE ATNA audit,
  per-version digital signatures, the FHIR R4 connectors, change events and
  the viewer are in one repository under the Business Source License 1.1.
  Nothing is held back to be sold back
  to you. (Vendored openEHR material keeps its own upstream terms, and the
  spec crates that embed it say so in their own metadata; see
  [Licensing & legal](licensing.md).)
- **Every claim checkable.** Conformance is executed by a runner against a
  live server, and the run records, per-case results, measured performance and
  the [comparison with another CDR](comparison.md) (in both directions) are
  committed to the repository. If a number appears on this site, the record it
  came from is in the tree, and a change that moves a verdict cannot land
  quietly.
- **The specification is the authority.** The normative openEHR text is
  vendored in the repository and cited decision by decision. Where we find it
  silent or self-contradictory, the finding is filed in public and reported
  upstream.
- **The specification layer as reusable libraries.** The generated openEHR
  model, the canonical codecs, the REST contract, the ADL engine and the AQL
  parser are published on crates.io as eight
  [standalone crates](crates.md), so the next Rust project does not have to
  re-model openEHR to get started. The five generated model crates are under
  the Apache License 2.0, the licence of the openEHR artifacts they are
  generated from, and need no licence conversation at all.
- **Maintenance in the open.** Public roadmap, public issue tracker,
  changelog-driven releases, signed artifacts, and a security policy with a
  private reporting channel.
- **A compliance posture you can read before you buy.** FerroEHR aims to be
  the first openly developed, source-available openEHR CDR with a published,
  tracker-backed EU compliance posture, and an EHDS conformity
  self-assessment is on its roadmap. The
  [compliance overview](compliance/index.md) states which controls ship
  today, which are planned and under which issue number, and which
  obligations stay with the organisation running the software. It claims no
  certification and no conformity, because a product cannot hold either on
  its own.

## Running FerroEHR commercially

FerroEHR is source-available under the Business Source License 1.1, which is
not an OSI-approved open-source licence. One licence covers the whole
repository, and no feature is held back for a paid tier.

| What you are doing | What you need | Why |
|---|---|---|
| Reading, building, modifying or redistributing the source | Free | The licence grants it without a fee and without asking anyone. |
| Development, testing, evaluation, prototyping | Free | All non-production use is granted. |
| Production use for Non-Commercial Purposes | Free | Personal use, academic or scientific research, teaching, and use by a non-profit organisation or public body that is not in the course of a business, does not deliver a service for payment, and is not for commercial advantage. |
| A hospital, clinic or care provider running it for its patients | Commercial licence | Delivering health care, or any other service for payment, is production use outside the grant. |
| A vendor or integrator, or any company running it in production | Commercial licence | Production use in the course of a business is outside the grant. |
| Offering it, or a work derived from it, to third parties as a hosted, managed or embedded service | Commercial licence | Excluded from the grant in every case, whoever you are. |
| Selling, sublicensing or otherwise distributing it for a fee | Commercial licence | Excluded from the grant in every case, whoever you are. |

The last two rows hold whatever else you are: they need a commercial licence
even for an organisation the rows above would otherwise leave free. Each
version becomes Apache 2.0 four years after it is published, so what ships
today opens up on a published schedule. [Licensing & legal](licensing.md)
carries the full picture, including every vendored third-party tree.

Building products on FerroEHR is welcome, and it is how standards reach
patients. Companies and care providers running FerroEHR are wanted here, and
the commercial licence is the normal path for them, on terms meant to make
those deployments and products possible on a sustainable footing. It is also
how the shared, conformant foundation gets maintained by the people who build
on it instead of every vendor rebuilding one in private. A commercial licence
is arranged with Cadasto B.V., the Licensor, which handles the business side
of FerroEHR: write to [info@cadasto.com](mailto:info@cadasto.com) or use
<https://www.cadasto.com/contact/>. Technical questions go to the maintainer
named in
[`MAINTAINERS.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/MAINTAINERS.md).
Talk to us early.

## What we ask in return

Contribute back.

The licence does not oblige you. The arithmetic does:

- **A private fork is the expensive option.** Fork it and you inherit the
  whole maintenance surface: specification releases, security advisories,
  database upgrades, re-running conformance, and re-merging your changes at
  every release, forever. Upstream the same change and it is maintained once,
  by everyone who runs it.
- **A defect found once should be fixed everywhere.** In clinical software
  the validation gap you patched privately is still live in every other
  deployment of the same code. Sharing the fix is the difference between one
  organisation being safe and all of them being safe.
- **Interoperability is a property of the population, not of any single
  implementation.** Every conformance case contributed, and every ambiguity
  resolved in the open, makes it likelier that your system and the next one
  actually agree about a record.

When the same fix is made privately in five places, the standard is no
stronger and five teams have paid for it. Nobody chose that outcome, and it is
an easy one to avoid.

## What contributing back can look like

You do not have to write Rust to make this project better:

- **A bug report** with the request that reproduces it, the most valuable
  thing most users will ever send.
- **A conformance case** for behaviour the catalogue does not cover yet, so
  the next release cannot regress it.
- **A specification finding**: the ambiguity you had to resolve in order to
  ship. We adjudicate those against the vendored text and report the genuine
  ones upstream to the openEHR Foundation.
- **Documentation**: a correction, deployment experience from your
  environment, or the paragraph that would have saved you two days.
- **Measurement** from your own hardware and workload. The instruments that
  produce our published numbers ship in the repository and run against any
  server, including one that is not FerroEHR.
- **Code**: fixes, features, connectors, packaging for your platform.
- **Sponsorship**, if your organisation depends on this and cannot spare
  engineering time.

[Contributing](contributing.md) is the practical starting point. Security
issues have their own private channel; please use it rather than the public
tracker.

## What this is not

This is not an argument against commercial or proprietary software. A product
with FerroEHR inside it, or a care provider running it for its patients, is a
good outcome, and we would rather those succeed than not exist at all; a
commercial licence is how they and this foundation support each other.

It is also not a claim to be the only good openEHR CDR. FerroEHR began as a
fork of EHRbase and records that lineage in the labelled import commit at the
root of its history; today it is an
independent Rust implementation with none of that code left in the tree, and
we measure ourselves against EHRbase with the same instrument, publishing both
directions of the result. FerroEHR is an independent implementation of the
openEHR® specifications and is not affiliated with or endorsed by the openEHR
Foundation.

And it is not a large team. The project is maintained by one person today,
with the machine gates (conformance, the fidelity suites, the CI guards)
standing in for review capacity a bigger group would have. The
[threat model](threat-model.md) and the repository's own governance and
maintainer documents say so plainly.

The same honesty covers how the code gets written: substantial parts of
FerroEHR are built with AI coding tools, directed and reviewed by the
maintainer, with every change held to the machine-enforced gates described
above. The full statement — what that means, what bounds it, and what you
can verify instead of trusting it — is the repository's
[AI statement](https://github.com/FerroHEALTH/FerroEHR/blob/main/AI_STATEMENT.md).

---

_Maintained by Ruben Talstra and the FerroEHR contributors. If your
organisation runs FerroEHR, or is thinking about building a product on it, we
would like to hear about it early: open an issue or say hello on the tracker._
