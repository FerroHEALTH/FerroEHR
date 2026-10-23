---
paths: ["app/ferroehr-viewer/**"]
---

# Leptos viewer rules (`app/ferroehr-viewer`)

**FerroUI leads the viewer's UI.** FerroUI (`ferrohealth-ui` on crates.io,
repository FerroHEALTH/FerroUI) is the family's component library and the
one source of its console design and its Leptos rules. Read these first,
before any viewer change:

- the consumer guide, [`docs/consoles.md`](https://github.com/FerroHEALTH/FerroUI/blob/main/docs/consoles.md):
  how a console takes the library, what it keeps local, how it moves to a
  new release;
- the component rules, [`.claude/rules/leptos-ui.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.3.2/.claude/rules/leptos-ui.md):
  the mandates (§0), the four-layer interaction contract (§1), reactivity,
  views, forms, tables and paging, accessibility to WCAG 2.2 AA, tokens and
  themes, icons, the bundle (§12), the catalogs (§13), the book's rules (§14)
  and the island rules (§15);
- the crate discipline, [`crates/ferrohealth-ui/CLAUDE.md`](https://github.com/FerroHEALTH/FerroUI/blob/main/crates/ferrohealth-ui/CLAUDE.md):
  the interaction contract, islands mode and the list of ready islands;
- the reviewed design, [`design/README.md`](https://github.com/FerroHEALTH/FerroUI/blob/main/design/README.md),
  [`design/COMPONENTS.md`](https://github.com/FerroHEALTH/FerroUI/blob/main/design/COMPONENTS.md)
  (the component IDs such as R9, F13 and E1 to E8, the family rules, the
  page templates) and the viewer's mockup under
  [`design/consoles/ferroehr/`](https://github.com/FerroHEALTH/FerroUI/tree/main/design/consoles/ferroehr).

**The pin decides the version you read.** The viewer pins `ferrohealth-ui =
"=0.3.2"` (root `Cargo.toml`), so the component rules, the guide, the crate
discipline and the design are read at the tag `v0.3.2`. The viewer runs in
islands mode under the four-layer interaction contract since the console
rollout (FerroHEALTH/FerroUI#161).
FerroUI's `design/` is the only design source: a design folder anywhere
else is stale and never synced from.

**Nothing FerroUI has or should have is built here.** A missing component,
a wrong behaviour in one, a stylesheet override or a script the viewer
would need is filed on FerroHEALTH/FerroUI (label `component`) and linked
from the viewer's issue; the viewer keeps its current part meanwhile and
takes the library's when the pin moves. Never a local copy, a shadow
component, a local stylesheet override or a hand-written script.

This file keeps only what is particular to the FerroEHR Viewer. The oracle
for every Leptos question FerroUI does not settle is the official Leptos
book (leptos-rs/book `main`, Leptos 0.8; `/leptos-lookup`), never memory.

## 0. Viewer mandates (absolute)

- **No hand-written JavaScript:** FerroUI's mandate (its `leptos-ui.md` §0)
  binds the viewer whole. The only JS is the generated `wasm-bindgen`
  bootstrap.
- **REST boundary:** the viewer reaches the CDR only over ITS-REST from its
  server (`reqwest`). It may depend on `crates/openehr-*`; it must NEVER
  depend on `app/ferroehr` or `app/ferroehr-rest`.
- **Every route and every `#[server]` fn is a public HTTP endpoint**
  (`server/25_server_functions`). A page is behind the login guard; a post is
  refused from another origin; a `#[server]` fn an island calls enforces the
  session itself and is declared
  `#[server(client = crate::session_client::SessionAwareClient)]`. CDR
  credentials never leave the server.

## 1. Two halves, chosen by target

- One crate, two halves: the server binary and the rlib on the host, the
  cdylib on `wasm32-unknown-unknown`. They are chosen by compilation target
  (`cfg(target_arch = "wasm32")`), never by a Cargo feature pair. Server-only
  dependencies sit under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`,
  and every module but the islands, the errors, the locale and the session
  client is server-only (`src/lib.rs`).
- WASM is 32-bit: never `usize`/`isize` in an island prop, a server-fn
  argument or any serialized shared type.
- **Views are built in `.into_any()`-erased sections**: plain `cargo` builds
  have no `erase_components`, and a monolithic `view!` blows rustc's layout
  recursion in `cargo test` codegen.
- The site is built with `--split` (the wrapper `scripts/cargo-leptos.sh`
  adds it): FerroUI's lazy islands import from a placeholder module that only
  the split rewrites, and without it no island hydrates (FerroUI #331).

## 2. Pages are drawn on the server

- Every route is `SsrMode::Async` and answers `GET` and `POST`: a post renders
  the page it was posted to, with `crate::posted::posted()` as the form. A
  page's reads are awaited in `pages::common::awaited` (a `Suspense` over one
  `Suspend`); never a `Resource` the browser re-runs.
- A form's default action is a hidden `default-intent`; a submit button's
  `name="intent"` wins (`Posted::intent`), whatever the order of the fields.
- A success redirects (`crate::flash::redirect_with`, a toast in a flash
  cookie); a refusal renders on the page posted to, the fields as typed.
- A page that docks its decision bar from inside an awaited future holds
  `crate::shell::promise_dock()` there. A docked form carries no `required`
  field: Discard posts the same form.
- A form never holds another form (the parser drops it); a second action in a
  form is a submit with `formaction`.
- A table row is one link (FerroUI R9); another control in the row sits in
  `.fui-table-actions` or the row menu.

## 3. Islands

- An island is the only part of a page the browser hydrates. Its props cross
  as serialized data, its words are resolved on the server (`*Words::of_page()`),
  and the page around it works without it: a control an island drives is a
  link or a submit until it hydrates (FerroUI's §15).
- An editor is editable from the first paint; never wait for the island to
  enable a field a page with no WebAssembly needs.
- The viewer's own islands live in `src/islands.rs`: the code editor, the
  template source upload, the subject reveal.

## 4. URL state and patient data

- Filter and search state lives in the URL (a `GET` form); paging of a
  listing that names no patient may too.
- **No patient identifier and no position in a listing that can show patient
  data goes in a URL** (FerroUI's mandate, its `leptos-ui.md` §0): those
  listings page through `crate::paging`, a session cookie per listing.

## 5. Theme, preferences and styles

- The theme and every other display preference is a cookie of its own
  (`crate::prefs` over FerroUI's `Preferences`), posted from the user menu and
  Your preferences and read when a page is drawn; no browser storage.
- FerroUI's stylesheet and its FerroEHR theme are served from this origin
  (`src/server.rs`). The viewer adds no stylesheet of its own.

## 6. Testing and gates

- FerroUI's own gates (its `leptos-ui.md` §10) hold the library; the
  design checks of the viewer's mockup (`design/COMPONENTS.md` § Checks every
  console runs, `design/tools/check.sh`) run in FerroUI. The viewer runs the
  gates below over its own code.
- Business logic lives out of components in plain types with unit tests.
- `tests/it/ssr_app.rs` drives every screen through the real service in
  process, over a CDR replayed from `tests/fixtures/cdr/recording.json`.
- **E2E is a merge gate**: `thirtyfour` driving headless Chromium against the
  composed stack (`scripts/ui-e2e.sh`), journeys in the `e2e_*` modules,
  skip-with-reason when `UI_E2E_BASE_URL` is unset. Every journey fails on a
  browser-console hydration error or panic. Explicit waits, never `sleep`; a
  journey holds the harness's bounded `common::Element`
  (`scripts/checks/e2e-waits.sh`). CI runs the battery at 1440 px and at
  390 px (`UI_E2E_VIEWPORT=phone`, device emulation), and `e2e_keyboard`
  drives the composition-level keyboard paths (#2793).
- Gates: `cargo clippy -p ferroehr-viewer --all-targets` on the host and
  `cargo clippy -p ferroehr-viewer --lib --target wasm32-unknown-unknown`,
  both `-D warnings`; `cargo nextest run -p ferroehr-viewer`; `leptosfmt` and
  `cargo fmt`; `bash scripts/cargo-leptos.sh build`, always the wrapper (a bare
  `cargo leptos` rewrites `Cargo.lock`).
- **The `ui-screenshot-guard` CI job**: a PR that touches
  `app/ferroehr-viewer/src/` commits refreshed captures under
  `website/book/src/viewer/img/` (`UI_E2E_DOCS_SHOTS=1 scripts/ui-e2e.sh`) or
  carries the `no-ui-visual-change` label (zero visual effect only). Applying
  the label raises a fresh run (#2777). The same pass captures every route at
  1440 and 390 px, light and dark, as the `ui-route-shots` artifact (#3910).

## 7. Bundle

`scripts/checks/viewer-bundle.sh` fails when the release WebAssembly,
brotli-compressed, is over the number below, which it reads from this table.

| Measure | Budget |
|---|---|
| WebAssembly, brotli-compressed | 450000 bytes |

The release build on 2026-10-23, on FerroUI 0.3.2 in islands mode with
`--split`, measured 416607 brotli bytes for the WebAssembly every page loads
(1425689 on FerroUI 0.1.4 with whole-page hydration); the lazy islands' chunks
load only on a page that draws them and are not counted. The budget leaves
about 8% for growth. Raise it only in the change that needs the room, with the
new measurement.

## 8. Strings and locales

The catalog rule is FerroUI's (its `.claude/rules/leptos-ui.md` §13): no
string the viewer draws lives outside a catalog, and a language joins as a
catalog file with no code change.

- The viewer's catalog is
  `app/ferroehr-viewer/locales/<tag>/ferroehr-viewer.ftl`, embedded by
  `fluent_templates::static_loader!` in `crate::locale`, where each message
  is one `Msg`. A test holds every catalog directory to the full message
  set, and a throwaway catalog under `tests/locales/` shows the words change
  with no code change.
- The server chooses the locale once per request (`crate::locale::current`):
  the best language of `Accept-Language` the catalog holds, else `[ui]
  locale`. The document writes it as `<html lang>`, the browser reads it back
  from there, and `App` provides it with `ferrohealth_ui::i18n::provide_locale`
  so the viewer's words and FerroUI's switch together.
- `scripts/checks/stray-literals.sh app/ferroehr-viewer/src` runs in CI
  against `scripts/checks/stray-literals.baseline`: a file may hold no more
  stray literals than its count, and a change that moves strings into the
  catalog lowers the count with `--update`. A literal no reader sees (an RM
  type name, a header token, a cookie's attributes) is allowed by pattern in
  `scripts/checks/stray-literals.allow`, with the reason.
