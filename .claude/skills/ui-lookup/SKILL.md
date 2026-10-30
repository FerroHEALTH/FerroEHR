---
name: ui-lookup
description: >
  Finds and reads the authoritative guidance for a viewer UI topic in
  FerroUI, which leads the UI (its consumer guide, component rules,
  migration guide and design with the component IDs, at the tag matching
  the pin), then in the HTML Living Standard, WAI-ARIA and WCAG 2.2 for
  what FerroUI does not settle. Use before implementing or reviewing any
  viewer (app/ferroehr-viewer) behaviour the rule file doesn't fully
  settle.
allowed-tools: [Read, Grep, Glob, Bash]
argument-hint: "<component / form / table / search / session / module topic>"
---

# /ui-lookup

Answer UI questions from the source text, never from memory. FerroUI leads
the viewer's UI, so its documents come first. The viewer's own rules live
in `.claude/rules/console-ui.md`.

## Procedure

1. **Read FerroUI first** (FerroHEALTH/FerroUI; a local checkout or
   GitHub). Take the tag from the viewer's pin (`ferrohealth-ui` in the root
   `Cargo.toml`):

   ```bash
   UI=~/.cache/ferroehr/ferroui
   [ -d "$UI/.git" ] || git clone https://github.com/FerroHEALTH/FerroUI "$UI"
   git -C "$UI" fetch --tags -q origin
   TAG="v$(sed -n 's/^ferrohealth-ui = "=\(.*\)"/\1/p' Cargo.toml)"
   git -C "$UI" show "$TAG:.claude/rules/console-ui.md"
   ```

   Route by topic: how a console takes the library, what stays local, the
   top bar's search wire, the session's end and how it moves to a release
   → `docs/consoles.md`; what changed from the Leptos releases →
   `docs/migration-0.6.md`; a component rule (templates, modules, forms,
   tables, accessibility, tokens, script budget, catalogs) →
   `.claude/rules/console-ui.md`; a component's arguments →
   `crates/ferrohealth-ui/src/view/`; what a page looks like, a component ID
   (R9, F13, E1) or a family rule → `design/COMPONENTS.md`,
   `design/README.md` and `design/consoles/ferroehr/`; how the gallery
   drives a component with the scripts loaded and refused →
   `app/ferrohealth-ui-gallery/tests/it/`. When FerroUI settles the
   question, stop here and cite the file and section.

2. **Read the standard** for what FerroUI does not settle: the HTML Living
   Standard (<https://html.spec.whatwg.org/>) for elements and forms,
   WAI-ARIA 1.2 and the Authoring Practices for roles and keyboard paths,
   WCAG 2.2 for success criteria, and the axum documentation on docs.rs at
   the pinned version for the server.

3. **Answer with citations** (file and section, or the standard's section).
   If the answer belongs in the standing rules, propose the addition
   explicitly: a library rule to FerroUI's `.claude/rules/console-ui.md` as
   a FerroUI issue, a viewer-only rule to this repository's
   `.claude/rules/console-ui.md`.
