# CRA Annex VII 4: the basis of the support period

CRA Annex VII point 4 (`docs/law/eu/cra/text.html`): "relevant information
that was taken into account to determine the support period pursuant to
Article 13(8) of the product with digital elements". CRA Art. 13(8), fifth
subparagraph: "Manufacturers shall include the information that was taken into
account to determine the support period of a product with digital elements in
the technical documentation as set out in Annex VII."

**The support period of each release is five years from the month it is
published**, stated in [`SECURITY.md`](../../SECURITY.md#supported-versions)
and, with its end date, in each release's notes.

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
- **Why not longer.** The security update for a release in its support
  period ships in the newest release, under CRA Art. 13(10), and every release
  is published under the same licence terms as the one before it, so the
  five-year period of a release is in practice met by the newest one. Whether
  Art. 13(10) holds for every commercial licence under BUSL-1.1 is a question
  for counsel (#3647).
- **The operating environment and integrated components** (Art. 13(8), second
  subparagraph, second sentence, which the manufacturer "may also take into
  account"). PostgreSQL, the distroless base image and the Rust crates each
  follow their own lifecycle; a fix to any of them reaches users in a newer
  FerroEHR release, so their support periods do not shorten FerroEHR's.

State: available. The running server states its own support end date
(#3639).
