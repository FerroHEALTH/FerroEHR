---
name: leptos-lookup
description: >
  Finds and reads the authoritative guidance for a viewer UI topic: first
  in FerroUI, which leads the UI (its consumer guide, component rules,
  crate discipline and design with the component IDs, at the tag matching
  the pin), then in the official Leptos book, cached locally (signals,
  effects, <For>, resources, Suspense/Transition, server functions,
  extractors, SSR modes, hydration bugs, router, ActionForm, islands,
  binary size). Use before
  implementing or reviewing any viewer (app/ferroehr-viewer) behaviour
  the rule file doesn't fully settle, or when a "how does Leptos do X"
  question comes up.
allowed-tools: [Read, Grep, Glob, Bash]
argument-hint: "<signal / resource / hydration / server fn / router topic>"
---

# /leptos-lookup

Answer UI questions from the source text, never from memory (Leptos is
pre-1.0 and moves fast; training data drifts). FerroUI leads the viewer's
UI, so its documents come first; the Leptos book answers what FerroUI does
not settle. The viewer's own rules live in `.claude/rules/leptos-ui.md`.

## Procedure

1. **Read FerroUI first** (FerroHEALTH/FerroUI; a local checkout or
   GitHub). Take the tag from the viewer's pin (`ferrohealth-ui` in the root
   `Cargo.toml`, `=0.2.0` today, tag `v0.2.0`):

   ```bash
   UI=~/.cache/ferroehr/ferroui
   [ -d "$UI/.git" ] || git clone https://github.com/FerroHEALTH/FerroUI "$UI"
   git -C "$UI" fetch --tags -q origin
   TAG="v$(sed -n 's/^ferrohealth-ui = "=\(.*\)"/\1/p' Cargo.toml)"
   git -C "$UI" show "$TAG:.claude/rules/leptos-ui.md"
   ```

   Route by topic: how a console takes the library, what stays local and
   how it moves to a release → `docs/consoles.md`; a component rule
   (reactivity, views, forms, tables, accessibility, tokens, bundle,
   catalogs) → `.claude/rules/leptos-ui.md`; the interaction contract,
   islands mode and the ready islands → `crates/ferrohealth-ui/CLAUDE.md`;
   what a page looks like, a component ID (R9, F13, E1) or a family rule →
   `design/COMPONENTS.md`, `design/README.md` and
   `design/consoles/ferroehr/`. A document missing at `$TAG` is read on
   `origin/main` (`git -C "$UI" show origin/main:<path>`), and the answer
   says which version it came from. When FerroUI settles the question,
   stop here and cite the file and section.

2. **Ensure the book cache exists** (shared, survives sessions):

   ```bash
   BOOK=~/.cache/ferroehr/leptos-book
   [ -d "$BOOK/src" ] || git clone --depth 1 https://github.com/leptos-rs/book "$BOOK"
   ```

   If the cache is older than ~30 days (`git -C "$BOOK" log -1 --format=%cr`),
   `git -C "$BOOK" pull --ff-only` first. The book's `main` targets the
   current Leptos 0.x line — cross-check any version-sensitive answer
   against the pinned Leptos version in the UI crate's `Cargo.toml`.

3. **Route to the owning chapter** (`src/SUMMARY.md` is the index):
   - signals / effects / memos / derived → `src/reactivity/*`,
     appendices `appendix_reactive_graph.md`, `appendix_life_cycle.md`
   - components / props / children / context → `src/view/03…09_*`,
     `src/interlude_projecting_children.md`
   - lists & keys → `src/view/04_iteration.md`, `04b_iteration.md`
   - control flow / errors → `src/view/06…07_*`
   - forms → `src/view/05_forms.md`; progressive enhancement →
     `src/progressive_enhancement/*`
   - async (Resource/Suspense/Transition/Action) → `src/async/*`
   - server fns / extractors / responses → `src/server/*`
   - SSR modes / page lifecycle / **hydration bugs** → `src/ssr/*`
   - router → `src/router/*`; global state → `src/15_global_state.md`
   - islands → `src/islands.md`; WASM size / deployment →
     `src/deployment/*`; styling → `src/interlude_styling.md`;
     head/meta → `src/metadata.md`; testing → `src/testing.md`;
     web_sys/JS interop → `src/web_sys.md`
4. **Grep** the exact API name (`Suspend`, `ForEnumerate`, `bind:value`,
   `extract_with_state`, …) across `src/**/*.md` when the routing isn't
   obvious.
5. **Read the surrounding section**, including the warnings/admonish blocks
   — the book's DON'Ts live there.
6. **Answer with citations** (`<chapter>.md` + heading). For API-signature
   detail beyond the book, follow its docs.rs links via Context7/WebFetch
   rather than guessing. If the answer belongs in the standing rules,
   propose the addition explicitly: a library rule to FerroUI's
   `.claude/rules/leptos-ui.md` as a FerroUI issue, a viewer-only rule to
   this repository's `.claude/rules/leptos-ui.md`.
