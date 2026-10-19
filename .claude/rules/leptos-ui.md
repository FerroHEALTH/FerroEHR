---
paths: ["app/ferroehr-viewer/**"]
---

# Leptos viewer rules (`app/ferroehr-viewer`)

**The component rules are FerroUI's.** The FerroHEALTH consoles share one
component library, FerroUI (`ferrohealth-ui` on crates.io, repository
FerroHEALTH/FerroUI), and one component rule set, its
`.claude/rules/leptos-ui.md`: the two tiers (static views and widgets),
render-mode agnostic components, reactivity and props, `<For>` keys, valid
HTML, forms, the one table and paging model, accessibility to WCAG 2.2 AA,
the role tokens and themes, Lucide icons, and the bundle. Read it before any
viewer change. This file keeps only what is particular to the FerroEHR
Viewer. The oracle for every Leptos question is the official Leptos book
(leptos-rs/book `main`, Leptos 0.8; `/leptos-lookup`), never memory.

## 0. Owner mandates (absolute)

- **Rust only, zero hand-written JavaScript.** No authored `.js`, no inline
  `<script>` bodies, no `onxxx="..."` attributes (the book's
  `oninput="this.form.requestSubmit()"` trick in `router/20_form` is
  forbidden; use an `on:` listener). The only JS is the generated
  `wasm-bindgen` bootstrap. JS-wrapping crates are banned.
- **REST boundary:** the viewer reaches the CDR only over ITS-REST from the
  BFF (server functions → `reqwest`). It may depend on `crates/openehr-*`;
  it must NEVER depend on `app/ferroehr` or `app/ferroehr-rest`.
- **Server functions are a public HTTP API** (`server/25_server_functions`).
  Every `#[server]` fn enforces the viewer's session itself and is declared
  `#[server(client = crate::session_client::SessionAwareClient)]` (the crate
  `CLAUDE.md`). CDR credentials never leave the server.
- **No component is built here that FerroUI should draw.** A component the
  viewer needs that FerroUI lacks is filed on FerroHEALTH/FerroUI (label
  `component`) and added there in a FerroUI release; the viewer keeps its
  current one meanwhile and moves to the library's when the pin moves.

## 1. Two halves, chosen by target

- One crate, two halves: the server binary and the rlib on the host, the
  cdylib on `wasm32-unknown-unknown`. They are chosen by compilation target
  (`cfg(target_arch = "wasm32")`), never by a Cargo `ssr`/`hydrate` feature
  pair, so a workspace `--all-features` build never enables both. Server-only
  dependencies sit under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.
- WASM is 32-bit: never `usize`/`isize` in a server-fn argument, return type
  or any serialized shared type (`server/25` quirks).
- Signals hold `Send + Sync` values; the `_local` variants only for genuinely
  `!Send` browser types.
- **Views are built in `.into_any()`-erased sections**: plain `cargo` builds
  have no `erase_components`, and a monolithic `view!` blows rustc's layout
  recursion in `cargo test` codegen.

## 2. Reactivity the viewer learned the hard way

- Writing one signal from an `Effect` that reads another is forbidden
  (`working_with_signals` §4). The two sanctioned async-answer-into-signal
  shapes: an Action's answer is written in the action's own continuation; a
  Resource seeding editable local state stays a one-shot resource-reading
  `Effect` with a written justification whenever any seed target renders
  outside the resource's own `Suspend` (seeding inside it writes during the
  server pass and again during hydration replay: an unrecoverable tachys
  hydration panic, reproduced live on `/queries/aql?load=…`).
- Seed-gated `disabled`: the live state goes on `prop:disabled`, with a
  static `disabled` attribute beside it for the server HTML.
- Toolchain trap (1.97, still live on 1.98.1): `use_query_map().with_untracked(…)`
  directly in a `#[component]` body stops `clippy::must_use_candidate` firing,
  so the crate-idiom `#[expect]` turns into an unfulfilled expectation. Put
  the URL read in a private helper fn (`ehrs.rs::find_from_url`).

## 3. Async data and hydration (the BFF pattern)

- Loading is a `Resource` over a `#[server]` fn, mutation an
  `Action`/`ServerAction`; reactive inputs go in the source. Lists that reload
  use `<Transition>`.
- **Never create a Resource (or render `<Outlet/>` or any resource-owning
  subtree) inside a `Suspend` closure**: it re-runs on every notification and
  re-creates everything inside, so server and client resource ids diverge
  (found live 2026-07-18). The shell renders the chrome and `<Outlet/>` once,
  outside its session `Suspense`.
- Do NOT use `<ErrorBoundary>` inside `<Suspense>` (a hydrated fallback
  mismatches in 0.8): resolve the `Result` inside the `Suspend` and render
  content or `notice::inline_error` as one erased view.
- Every authenticated route uses `SsrMode::Async` on the parent route
  (streamed fragments race WASM init); `/login` too, so it works without
  JavaScript.
- Anchors to BFF axum routes carry `rel="external"`.

## 4. URL state and patient data

- Filter, search and paging state of a listing that names no patient lives in
  the URL (`<Form method="GET">` → `use_query_map` → resource source).
- **No patient identifier and no position in a listing that can show patient
  data goes in a URL**: those listings page with
  `components::data_table::paging_in_session` (FerroUI's session pager).

## 5. Theme and styles

- The theme is the `ferroehr-theme` cookie the switch posts to `/theme`,
  written by the server as `data-theme` on `<html>` (`crate::theme`); no
  browser storage.
- FerroUI's stylesheet and `style/theme.css` (FerroEHR's palette over
  FerroUI's roles) are served from this origin. The viewer's Tailwind
  utilities name roles only (`bg-raised`, `text-ink-muted`, `border-edge`),
  never a palette entry and never a per-element `dark:`.
- `tests/it/contrast.rs` holds every pairing of the FerroEHR theme to WCAG 2.2
  AA; a new colour joins it in the change that adds it.

## 6. Testing and gates

- Business logic lives out of components in plain types with unit tests.
- SSR render tests (`tests/it/ssr_*.rs`) render with `RenderHtml::to_html`
  in an `Owner`.
- **E2E is a merge gate**: `thirtyfour` driving headless Chromium against the
  composed stack (`scripts/ui-e2e.sh`), journeys in the `e2e_*` modules,
  skip-with-reason when `UI_E2E_BASE_URL` is unset. Every journey fails on a
  browser-console hydration error or panic. Explicit waits, never `sleep`; a
  journey holds the harness's bounded `common::Element`
  (`scripts/checks/e2e-waits.sh`).
- Gates: `cargo clippy -p ferroehr-viewer --all-targets` on the host and
  `cargo clippy -p ferroehr-viewer --lib --target wasm32-unknown-unknown`,
  both `-D warnings`; `cargo nextest run -p ferroehr-viewer`; `leptosfmt` and
  `cargo fmt`; `bash scripts/cargo-leptos.sh build`, always the wrapper (a bare
  `cargo leptos` rewrites `Cargo.lock`).
- **The `ui-screenshot-guard` CI job**: a PR that touches
  `app/ferroehr-viewer/src/` or `style/` commits refreshed captures under
  `website/book/src/viewer/img/` (`UI_E2E_DOCS_SHOTS=1 scripts/ui-e2e.sh`) or
  carries the `no-ui-visual-change` label (zero visual effect only). Applying
  the label raises a fresh run (#2777).

## 7. Bundle

`scripts/checks/viewer-bundle.sh` fails when the release WebAssembly,
brotli-compressed, is over the number below, which it reads from this table.

| Measure | Budget |
|---|---|
| WebAssembly, brotli-compressed | 1550000 bytes |

The release build on 2026-10-19, on FerroUI 0.1.4 from crates.io with `thaw`, `leptos-use`
and `leptos-chartistry` gone, measured 1425689 brotli bytes of WebAssembly
(1433578 on 0.1.3);
the budget leaves about 8% for growth. Raise it only in the
change that needs the room, with the new measurement.
