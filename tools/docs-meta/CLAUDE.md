# `docs-meta`: the documentation site's search metadata (tooling, not part of the app)

One binary that rewrites the assembled site in place: every book page under
`<site>/docs/<version>/` gets a canonical link to its `latest` copy, a
description from the page's own opening prose, Open Graph and Twitter card
tags, `noindex,follow` on every version other than `latest`, and a
`BreadcrumbList`. `scripts/site/build.sh` runs it as
`docs-meta <site-dir> <origin> <base-path>` in both modes, which the Docs
workflow and the external link check call.

- **std only.** `[dependencies]` stays empty: it runs inside the docs
  workflow, and every dependency is one more way a documentation deploy can
  break. A need for a crate is a design question for the issue, never a
  quiet addition.
- **It works on the built HTML.** mdBook's template context carries only a
  chapter's source path, so the theme cannot form the absolute canonical URL,
  and crawlers do not reliably honour a canonical added by JavaScript.
- **Idempotent.** A page that already carries the `MARKER` comment is
  skipped, so a second run over the same tree changes nothing. Keep that
  property for every new tag it writes.
- **Deterministic.** Versions and pages are walked in sorted order, so the
  same tree yields the same output.
- **Never shipped.** No `app/*` crate depends on it, and it depends on none;
  the console is its interface, which is why `print_stdout` and
  `print_stderr` are allowed at the crate root with a reason.
- **Gates:** `cargo clippy -p docs-meta --all-targets -- -D warnings`,
  `cargo nextest run -p docs-meta`, and `bash scripts/site/build.sh
  --dev-only` when the output shape changes.
