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
- the component rules, [`.claude/rules/leptos-ui.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.5.3/.claude/rules/leptos-ui.md):
  the mandates (§0), the four-layer interaction contract (§1), reactivity,
  views, forms, tables and paging, accessibility to WCAG 2.2 AA, tokens and
  themes, icons, the bundle (§12), the catalogs (§13), the book's rules (§14)
  and the hydration rules (§15);
- the crate discipline, [`crates/ferrohealth-ui/CLAUDE.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.5.3/crates/ferrohealth-ui/CLAUDE.md):
  the interaction contract, the whole page hydrated and the list of ready
  components;
- the reviewed design, [`design/README.md`](https://github.com/FerroHEALTH/FerroUI/blob/main/design/README.md),
  [`design/COMPONENTS.md`](https://github.com/FerroHEALTH/FerroUI/blob/main/design/COMPONENTS.md)
  (the component IDs such as R9, F13 and E1 to E8, the family rules, the
  page templates) and the viewer's mockup under
  [`design/consoles/ferroehr/`](https://github.com/FerroHEALTH/FerroUI/tree/main/design/consoles/ferroehr).

**The pin decides the version you read.** The viewer pins `ferrohealth-ui =
"=0.5.3"` (root `Cargo.toml`), so the component rules, the guide, the crate
discipline and the design are read at the tag `v0.5.3`. The viewer hydrates
the whole page and draws every page after the first in the browser, under
the four-layer interaction contract (FerroHEALTH/FerroUI#410, #4116).
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
  refused from another origin; every `#[server]` fn enforces the session
  itself (a page's model through `crate::request::visiting`) and is declared
  `#[server(client = crate::session_client::SessionAwareClient)]`. CDR
  credentials never leave the server.

## 1. Two halves, chosen by target

- One crate, two halves: the server binary and the rlib on the host, the
  cdylib on `wasm32-unknown-unknown`. They are chosen by compilation target
  (`cfg(target_arch = "wasm32")`), never by a Cargo feature pair. Server-only
  dependencies sit under `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`.
  The app, the routes, the shell, the pages and every module they draw from
  compile on both halves; the server-only modules are the ones `src/lib.rs`
  gates (`admin`, `aql_text`, `cdr`, `chart_model`, `config`, `download`,
  `export`, `feedback`, `manufacturer`, `oidc`, `queries_api`, `server`,
  `served_vocabulary`, `session`, `state`, `system_api`), and a server-only
  helper inside a shared module carries its own `cfg`.
- **The request state reaches both halves through shared values.** The
  server reads the path, the query, the display cookies, who is signed in,
  the viewer's settings and the instant it draws the page at once per page
  into `crate::request::Visit`; the app provides it, the posted form
  (`crate::posted::provide_posted`) and the flash toast
  (`crate::flash::shared`) through `SharedValue`, and the status document
  through an `ArcOnceResource` (`crate::shell::Status`). A view reads them
  through the same functions on both halves (`request::path`,
  `request::query`, `request::signed`, `request::settings`, `request::now`,
  `prefs::current`, `paging::current`, `posted::posted`,
  `routes::params`), and never reads the clock itself; `request::session`,
  `request::state`, `set_status` and `set_cookie` stay on the server. The
  visit carries no session cookie and no credential. In the browser the route
  group sets the visit's address before it draws a page
  (`request::follow_address`), `request::FollowVisit` asks the server for the
  rest after each navigation and post, and the posted form lets go once the
  browser draws another page.
- WASM is 32-bit: never `usize`/`isize` in an island prop, a server-fn
  argument or any serialized shared type.
- **Views are built in `.into_any()`-erased sections**: plain `cargo` builds
  have no `erase_components`, and a monolithic `view!` blows rustc's layout
  recursion in `cargo test` codegen.
- The site is built with `--split` (the wrapper `scripts/cargo-leptos.sh`
  adds it): the lazy route groups import from a placeholder module that only
  the split rewrites, and without it nothing hydrates.

## 2. Pages: the server draws the first, the browser every page after it

- The pages sit in six lazy groups (`src/routes.rs`): `routes::ROUTES` is the
  one route table, the shell's one route (`routes::Pages`) loads the group
  that holds the page the address names, and the group draws that page again
  as the address changes. A new page joins `ROUTES`, `routes::Page` and its
  group. A route parameter is read with `routes::params()`, never
  `use_params_map`, since the router holds one route.
- The server answers every document `SsrMode::Async` and every route takes
  `GET` and `POST`: a post renders the page it was posted to, with
  `crate::posted::posted()` as the form. A page's model is one server function
  (`load_…`, under `page_model!`) around a server-only `read_…`: the server's
  render calls it in place and the browser asks for it after a navigation.
  It is drawn with `pages::common::drawn` or `docked` (a `Suspense` over one
  `Suspend`); never a `Resource` the browser re-runs.
- A link and a `GET` form navigate in the page (FerroUI's `FormNavigator`);
  a viewer post loads a new document, the server's answer; FerroUI's
  preference, theme, rail and pager forms post in the background. A link to
  a route that is no page carries `rel="external"` or `download`
  (FerroUI's `Act::Download`), and an export submit FerroUI's `RELOADS`.
- A form's default action is a hidden `default-intent`; a submit button's
  `name="intent"` wins (`Posted::intent`), whatever the order of the fields.
- A success redirects (`crate::flash::redirect_with`, a toast in a flash
  cookie); a refusal renders on the page posted to, the fields as typed.
- A page that docks its decision bar from inside an awaited future holds
  `crate::shell::promise_dock()` there. Discard leaves with `formnovalidate`
  (FerroUI's `Decision::leaves`), so a docked form may mark a field
  `required`; the save checks it again.
- A form never holds another form (the parser drops it); a second action in a
  form is a submit with `formaction`.
- A table row is one link (FerroUI R9); another control in the row sits in
  `.fui-table-actions` or the row menu.

## 3. Hydration

- The browser entry hydrates the whole page with FerroUI's
  `widget::hydration::hydrate`, which writes `data-ferroui-hydrated` on the
  root element once the page and its lazy group have hydrated and leaves
  every click and submit to the browser until then. Nothing changes a node
  the server drew before that mark, and a browser journey waits for it.
- A page works without the WebAssembly: a control a component drives is a
  link or a submit until the page hydrates (FerroUI's §15).
- An editor is editable from the first paint; never wait for hydration to
  enable a field a page with no WebAssembly needs.
- The viewer's own components live in `src/components.rs`: the code editor,
  the template source upload, the subject reveal.

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
  carries the `no-ui-visual-change` label (zero visual effect only). It reads
  only the PR's own commits, from the merge base
  (`scripts/checks/ui-screenshot.sh`, #4117). Applying the label raises a
  fresh run (#2777). The same pass captures every route at
  1440 and 390 px, light and dark, as the `ui-route-shots` artifact (#3910).

## 7. Bundle

`scripts/checks/viewer-bundle.sh` fails when the release WebAssembly,
brotli-compressed, is over the number below, which it reads from this table.

| Measure | Budget |
|---|---|
| WebAssembly, brotli-compressed | 450000 bytes |

The release build on 2026-10-27, on FerroUI 0.5.2 with the whole page
hydrated, the pages in six lazy groups and `--split`, measured 411846
brotli bytes for the entry WebAssembly every page loads (449938 before the
catalog split, 399352 for the shell alone with stub pages, 425829 on FerroUI
0.3.6 in islands mode); a group's chunks load with its pages and are not
counted. The entry carries the shell, the router and the shell's catalog.
What keeps it in budget: one route for the router (`routes::Pages`, one
erased `Suspend` for every group), the page models read as server functions
from the groups alone, the shell reading its status on the server alone, a
catalog for each route group that loads with the group's chunks (§8), and
`SPLIT_ROOM` (`src/lib.rs`, #4225), without which the splitter leaves every
page's read-only data in the entry (474943). Raise the budget only in the
change that needs the room, with the new measurement.

## 8. Strings and locales

The catalog rule is FerroUI's (its `.claude/rules/leptos-ui.md` §13): no
string the viewer draws lives outside a catalog, and a language joins as a
catalog file with no code change.

- The viewer's catalogs are
  `app/ferroehr-viewer/locales/<part>/<tag>/<part>.ftl`, one for each part
  that looks its words up, embedded by `fluent_templates::static_loader!`:
  `shell` holds the words of the shell and of more than one group
  (`crate::locale::Msg`), each route group has its own (`overview`,
  `records`, `definitions`, `queries`, `console`, `signin`, each with its
  message type in `crate::locale::<part>`), and `server` holds the words
  only the server draws. A catalog is reached only through its message type,
  so the browser loads a group's words with the group's chunks and the entry
  carries the shell's catalog alone. A message goes in the catalog of the
  one group that draws it, else the shell's; a message no code draws is
  removed. A test holds every part's catalog directories to its full message
  set in the shell's languages, and a throwaway catalog under
  `tests/locales/` shows the words change with no code change.
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
