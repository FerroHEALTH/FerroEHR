# CRA Annex VII 4: the basis of the support period

CRA Annex VII point 4 (`docs/law/eu/cra/text.html`): "relevant information
that was taken into account to determine the support period pursuant to
Article 13(8) of the product with digital elements". CRA Art. 13(8), fifth
subparagraph: "Manufacturers shall include the information that was taken into
account to determine the support period of a product with digital elements in
the technical documentation as set out in Annex VII."

**Only the newest release is supported.** When a new release is published,
every earlier release stops receiving fixes: there are no maintenance
branches, no long-term-support line and no backports. If you are not on the
newest release, upgrade; upgrading is free. Under the Cyber Resilience Act,
Cadasto B.V. commits to keep publishing security fixes for at least five years
after each release (Art 13(8)) and ships them only in the newest release
(Art 13(10)). The policy is stated in
[`SECURITY.md`](../../SECURITY.md#supported-versions), and the end month of
the commitment for a release is printed in its notes (owner decision
2026-10-19, #3853).

The support period of Art. 13(8) is therefore a commitment for the product:
for at least five years after each release is placed on the market, Cadasto
B.V. keeps handling vulnerabilities and publishing fixed releases. Art.
13(10) lets those security updates be delivered only in the version last
placed on the market, provided users of earlier versions receive it free of
charge and with no added cost to adjust their hardware and software
environment; every FerroEHR release is published under the same licence terms
as the one before it, and an upgrade needs no new environment.

## What was taken into account

CRA Art. 13(8), second subparagraph, has the support period "reflect the
length of time during which the product is expected to be in use, taking into
account, in particular, reasonable user expectations, the nature of the
product, including its intended purpose, as well as relevant Union law
determining the lifetime of products with digital elements". The third
subparagraph sets "at least five years" unless the product is expected to be
in use for less.

- **The expected time in use.** Longer than five years. FerroEHR holds the
  health records of a healthcare provider
  ([intended purpose](../../website/book/src/compliance/intended-purpose.md#the-operational-environment)),
  and a provider replaces its record system rarely and at the cost of a
  migration of every record.
- **Reasonable user expectations, and the nature of the product.** The records
  outlive any one release: the information on each access to them stays
  available for at least three years (EHDS Art. 9(2)), and national law keeps
  clinical records and access logs longer, which the retention register
  records per jurisdiction ([retention](../../website/book/src/compliance/retention.md)). A provider
  expects the system holding them to stay supported for years, so the minimum
  of five years applies and nothing argues for less.
- **Relevant Union law determining the lifetime.** The vendored texts set no
  lifetime for an EHR system or a clinical data repository.
- **Why not longer.** Every security update ships in the newest release,
  under CRA Art. 13(10), and every release is published under the same
  licence terms as the one before it, so a commitment that restarts with each
  release covers a provider that keeps upgrading for as long as FerroEHR is
  published. Whether Art. 13(10) holds for every commercial licence under
  BUSL-1.1 is a question for counsel (#3647).
- **The operating environment and integrated components** (Art. 13(8), second
  subparagraph, second sentence, which the manufacturer "may also take into
  account"). PostgreSQL, the distroless base image and the Rust crates each
  follow their own lifecycle; a fix to any of them reaches users in a newer
  FerroEHR release, so their support periods do not shorten FerroEHR's.

State: available. The running server states the end month of the commitment
for its release (#3639).
