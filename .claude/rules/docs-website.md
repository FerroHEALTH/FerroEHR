---
paths: ["website/**", "scripts/site/build.sh", "scripts/site/cut-version.sh", ".github/workflows/docs.yml", ".github/workflows/release.yml"]
---

# The documentation website (`website/**`)

<!-- Path-scoped 2026-07-13: the global "same-PR docs" reminder lives in the
     root CLAUDE.md hard rules; this file carries the detail and loads when
     website files are touched. -->

The public site — https://ferroehr.eu/ — is built from
`website/` by `.github/workflows/docs.yml` and deployed to GitHub Pages.
No openEHR spec governs the website — our own design; this rule is the
authority for layout, look & feel, and content.

## Layout & URLs

- `website/landing/` — the hand-written landing page (root `/`). Relative
  URLs only; light+dark via `prefers-color-scheme`; zero external runtime
  requests.
- `website/book/` — the mdBook (served at `/docs/dev/`, frozen per release at
  `/docs/vX.Y.Z/`, newest release copied to `/docs/latest/`).
- Frozen versions live on the `docs-dist` orphan branch (generate once,
  never rebuilt); `scripts/site/cut-version.sh vX.Y.Z` cuts one (the release
  pipeline's docs-freeze leg does this on every non-prerelease `v*` tag).

## Family documentation lives in the FerroHEALTH book (#3701)

FerroEHR is one product of the FerroHEALTH family, and Cadasto B.V. is the
manufacturer of every product in it. Documentation about the family or the
manufacturer is written ONCE, in the FerroHEALTH book
(`https://ferrohealth.eu/docs/`, repo FerroHEALTH/FerroHEALTH), and this book
links it. The family pages, at fixed addresses:

| Family page | Address | Kept here as a short page linking it |
|---|---|---|
| The manufacturer | `manufacturer.html` | (cited from `compliance/cra-user-information.md` point 1, `compliance/information-sheet.md` (a)) |
| Security: vulnerability handling, disclosure, advisories, security-only patches, CRA Art 14 reporting, cessation | `security.html` | `SECURITY.md` keeps the supported versions, the reporting routes and scope, the repository settings, `security.txt` |
| Post-market procedure | `post-market.html` | `compliance/post-market.md` (FerroEHR's channels, registers, withdrawn releases); `docs/post-market.md` (the steps bound to this repository's registers, tracker, release lane and `scripts/release/withdraw.sh`) |
| The CRA, the manufacturer side | `cra.html` | `compliance/cra.md` (FerroEHR's product facts, its CRA documents, the class I question) |
| The EHDS EHR system: intended purpose | `ehds/intended-purpose.html` | `compliance/intended-purpose.md` (FerroEHR's essential functions, data, environment, security environment, foreseeable misuse) |
| The EHDS EHR system: shared responsibility | `ehds/shared-responsibility.html` | `compliance/shared-responsibility.md` (the usage report, where FerroEHR documents its side) |
| Licensing and trademarks | `licensing.html` | `licensing.md` (installing a licence, the crate licences, vendored material, REUSE, lineage; `licensing-declarations.sh` and `copyright-holder.sh` read it) |
| How the products fit together | `architecture.html` | none |

**Stays in this book, linking the family book:** the information sheet and
the instructions for use (the release lane attaches them to every release),
the generated EHDS readiness and technical-documentation pages
(`website/book/ehds.yaml` + `scripts/render/ehds-pages.sh`), FerroEHR's CRA
risk assessment and Annex II user information, the hazard log, the claims
review, the control matrix, the compliance overview, and every installation,
configuration, API, operations and security page. A moved page keeps its file
and URL so frozen books and the release notes' links keep resolving. Before
writing a page here, ask whether it is about FerroEHR alone; if it is about
the family or the manufacturer, it goes to the FerroHEALTH book.

## The same-PR docs rule (mirror of the changelog rule)

**Any PR that changes the REST surface, configuration (`FERROEHR_*`), the
CLI, deployment artifacts (compose/Helm/containers), or other user-visible
behaviour must update the matching `website/book/src` page in the same PR.**
The `/phase-done` checklist enforces it at phase close.

## Authoring rules

- End-user voice: second person, task-first. Audiences are integrators,
  operators, and evaluators — never repo-internal.
- **Never publish or link:** `docs/plans/**`,
  `docs/specs/**`, `.claude/**`. Paraphrase into user language instead.
- Callouts: mdBook 0.5 native `> [!NOTE]` / `[!WARNING]` / `[!TIP]` (no
  plugin syntax). Diagrams: ```mermaid fenced blocks. Long chapters get
  `<!-- toc -->` after the intro paragraph.
- **Math: `$…$` inline, `$$…$$` display** (mdbook-katex, pre-rendered at
  build time — no client JS, no CDN; the KaTeX CSS/fonts are self-hosted at
  `website/book/src/katex/`, version-locked to the renderer — see
  `website/book/KATEX-PROVENANCE.md`). Fenced code and inline code are never
  processed, so shell `$VAR` and Argon2 `$argon2id$…` strings are safe; a
  literal dollar in PROSE must be escaped `\$`. Use math for actual
  formulas (arrival schedules, derivations, bounds), not for every symbol —
  a ratio like 10:1 reads fine as prose.
- Every endpoint, header, status code, and config key must be verified
  against the code or the vendored OAS before it is written down.

## What a gate checks, and what only review can (#1984)

The v3.17.4 book sweep found five substantive defects that had already shipped
to the published site, and no gate had caught any of them: `mdbook-lint`
checks style, `lychee` checks that links resolve, and both pass on a page
whose every technical claim is false. Per the reliability convention that an
unenforced rule is labelled as one, here is the honest split.

**Machine-checked** — `scripts/checks/docs-claims.sh`, the `docs-claims` CI
job, `--all` over the whole book:

- Every `FERROEHR__…` environment form resolves to a key in
  `app/ferroehr/assets/ferroehr.default.toml`.
- Every Helm values path on a chart page resolves — against `values.yaml`, or
  for `config.*` against the TOML schema, since the chart renders `config:`
  verbatim into `ferroehr.toml`.
- Every committed chart under a `*-assets/` directory is embedded by a page or
  by an mdBook `{{#include}}` source.
- Every version literal that is a claim about the current release: a chart
  `--version` against `Chart.yaml`'s own version, an `image.tag=`/`ghcr.io/…:`
  tag against the WORKSPACE version (not the chart's `appVersion`, which is
  injected at publish time and may lag in the tree — #2779), and on
  `verifying-releases.md` the release-asset filenames and the
  substitute-this-tag note.

**Review-only, deliberately:**

- **Documented Rust paths.** A grep-level existence check for `crate::a::b` /
  `openehr_x::y::Z` was measured and rejected: prose names types under a
  different module path than the sentence implies, and the generated crates'
  generation modules give one type several valid paths. It found less than it
  cried wolf about.
- **Prose claims** — "six endpoints exist", "the chart renders one key",
  behaviour descriptions of any kind. No machine authority exists for these,
  which is exactly why the same-PR rule above still carries the weight.

A guard is only worth having if it is trusted, so when `docs-claims` reports
something, check whether the *guard* is wrong before editing the page: its
first run reported a published chart as orphaned because it searched only
`website/book/src` and the reference lived in `website/book/generated`.

## The API reference is the live sandbox

The site hosts no OpenAPI reference of its own. Every "API reference" link
points at the hosted sandbox's Swagger UI,
<https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui>, which the server
generates from its own handlers and serves at the running release. The demo
credentials are `ferroehr` / `ferroehr`
(`website/book/src/installation/codespaces.md`). Every deployment serves the
same UI at `/ferroehr/rest/swagger-ui` when `swagger_ui` is on, so a reader
running their own server reads their own surface.

## Never hand-edit

- `website/book/theme/mermaid*` — vendored assets.

## Toolchain

Pinned in `.github/actions/docs-toolchain` (mdBook, mdbook-mermaid, mdbook-toc, mdbook-lint,
lychee). Bump only after live-verifying the new version.

## Local preview

`bash scripts/site/build.sh` then serve `_site/`
(`npx --yes serve _site`, or any static server). CI equivalence is the point — if it
works locally it works deployed.
