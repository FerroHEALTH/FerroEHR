# Mermaid assets (vendored)

`theme/mermaid.min.js` and `theme/mermaid-init.js` are the browser assets of
`mdbook-mermaid` **0.17.0**, the version `.github/actions/docs-toolchain`
installs, taken verbatim from
<https://github.com/badboy/mdbook-mermaid> at commit
`619ed3c601728640608dd9b0bcd9935157fac24c` (tag `v0.17.0`),
`src/bin/assets/`. `mermaid.min.js` is the mermaid **11.6.0** browser bundle.

| File | sha256 | Licence |
|---|---|---|
| `theme/mermaid.min.js` | `eefea253bed9655e838eb874ff955c46872f982a8e26290c1dd2982ddc0a4703` | MIT (mermaid) |
| `theme/mermaid-init.js` | `ccf746f10c0a71bd34799867d2f9860dddb5fc6efc6c50d1ea55a1eb1a7bd406` | MPL-2.0 (mdbook-mermaid) |

The book serves both files itself, so no page fetches a script from another
origin. Bump them with `mdbook-mermaid`: run `mdbook-mermaid install
website/book` at the new pin, move the two files into `theme/`, and update
this table. FerroBRIDGE, FerroFED, FerroPIX, FerroSMART and FerroTERM serve
the same `mermaid.min.js`.
