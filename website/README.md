# FerroEHR documentation website

This directory holds the public documentation site published to
**https://ferroehr.eu/** (GitHub Pages, custom apex domain — the site is
served at the domain root). The site conventions — toolchain pins, URL scheme,
versioning machinery, design tokens, content map — are documented in
`.claude/rules/docs-website.md` and this file.

## Layout

| Path | What |
|---|---|
| `landing/` | Hand-written static landing page (`index.html` + `style.css`, `404.html`, `robots.txt`, `assets/`). Relative URLs only. |
| `book/` | The mdBook (`book.toml`, `src/`, additive `theme/`). Built per deployment path with the matching `site-url`. |
| `versions.json` | Version manifest the picker and deploy job read. |

There is no OpenAPI reference in this tree. The API reference is the hosted
sandbox's own Swagger UI,
<https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui>, which the running
server generates from its handlers.

## Building locally

The build needs a Rust-only toolchain: `mdbook`, `mdbook-mermaid`, `mdbook-toc`
(pins in `docs.yml` / the phase file §1). Then:

```shell
bash scripts/site/build.sh --dev-only # assemble ./_site (landing + /docs/dev/)
# serve _site/ with any static server, e.g.:  npx --yes serve _site
```

`--full` additionally materializes the frozen versions from the `docs-dist`
branch plus `/docs/latest/`, exactly as CI does before deploy.

## Rules

- Never hand-edit the vendored `book/theme/mermaid*` assets; their pin and
  sha256 are in `book/MERMAID-PROVENANCE.md`.
- Authoring voice is end-user and task-first; never publish internal trees
  (plans, rules, the vendored spec oracle).
