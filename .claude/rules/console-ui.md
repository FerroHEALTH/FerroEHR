---
paths: ["app/ferroehr-viewer/**"]
---

# Viewer console rules (`app/ferroehr-viewer`)

**FerroUI leads the viewer's UI.** FerroUI (`ferrohealth-ui` on crates.io,
repository FerroHEALTH/FerroUI) is the family's component library and the
one source of its console design and its component rules. Read these first,
before any viewer change:

- the consumer guide, [`docs/consoles.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.6.0/docs/consoles.md):
  how a console takes the library, what it keeps local, the top bar's
  search wire, the session's end and how it moves to a new release;
- the move from the Leptos releases,
  [`docs/migration-0.6.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.6.0/docs/migration-0.6.md);
- the component rules, [`.claude/rules/console-ui.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.6.0/.claude/rules/console-ui.md):
  the mandates (§0), the interaction contract (§1), templates and markup,
  modules, forms, tables and paging, accessibility to WCAG 2.2 AA, tokens
  and themes, testing (§10), the script budget (§12) and the catalogs (§13);
- the reviewed design, [`design/README.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.6.0/design/README.md),
  [`design/COMPONENTS.md`](https://github.com/FerroHEALTH/FerroUI/blob/v0.6.0/design/COMPONENTS.md)
  (the component IDs such as R9, F13 and E1 to E8, the family rules, the
  page templates) and the viewer's mockup under
  [`design/consoles/ferroehr/`](https://github.com/FerroHEALTH/FerroUI/tree/v0.6.0/design/consoles/ferroehr).

**The pin decides the version you read.** The viewer pins `ferrohealth-ui =
"=0.6.0"` (root `Cargo.toml`), so every document above is read at the tag
`v0.6.0`. FerroUI's `design/` is the only design source: a design folder
anywhere else is stale and never synced from.

**Nothing FerroUI has or should have is built here.** A missing component,
a wrong behaviour in one, a stylesheet override or a script the viewer
would need is filed on FerroHEALTH/FerroUI (label `component`) and linked
from the viewer's issue; the viewer keeps its current part meanwhile and
takes the library's when the pin moves. Never a local copy, a shadow
component, a local stylesheet override or a hand-written script.

This file keeps only what is particular to the FerroEHR Viewer. The oracles
for markup and behaviour are the HTML Living Standard, CSS and WCAG 2.2,
and for the server the axum documentation at the pinned version, never
memory.

## 0. Viewer mandates (absolute)

- **Server-rendered HTML, FerroUI's modules and nothing else.** The server
  draws every page as one HTML document. The only script is FerroUI's
  loader and its modules, served from `/ferrohealth-ui/<version>/scripts/`;
  the viewer writes no JavaScript, no inline `<script>` body, no `on*=`
  attribute, no `style` attribute and no `<style>` element. The policy is
  `script-src 'self'; style-src 'self'` with no nonce (`server::POLICY`).
- **REST boundary:** the viewer reaches the CDR only over ITS-REST from its
  server (`reqwest`). It may depend on `crates/openehr-*`; it must NEVER
  depend on `app/ferroehr` or `app/ferroehr-rest`.
- **Every route is a public HTTP endpoint.** A page is behind the login
  guard (`server::login_guard`); a request that is not a safe method passes
  `ferrohealth_server::same_origin` first; every read of the CDR calls
  `crate::session::require_session` itself. CDR credentials never leave the
  server.

## 1. The request scope

- **A page handler draws inside one `crate::scope::Scope`**, a Tokio
  task-local holding the viewer's state, the request's parts, the posted
  form, the locale, the instant the page is drawn at, the status document
  read at most once, and the answer the page builds (its status, its
  redirect, the cookies it sets, its docked bar, and whether a read met the
  session's end). A page reads them through `crate::request` (`path`,
  `query`, `signed`, `settings`, `now`, `session`, `state`),
  `crate::posted::posted`, `crate::locale::current`, `crate::prefs::current`,
  `crate::paging::current` and `crate::routes::params`, and never reads the
  clock itself.
- A page answers through the same scope: `request::set_status`,
  `request::redirect` (a `302`, which a later `set_status` never
  overwrites), `request::set_cookie` and `shell::dock`. A future spawned
  onto another task leaves the scope, so every read of a page runs on the
  handler's own task.
- **A dead session ends the page.** `require_session` refusing and a CDR
  `401` both note it in the scope (`session_client::note_if_over`); the
  page handler then answers with the sign-in page
  (`session_client::signed_out_url`) instead of a shell over a dead session
  (FerroUI `docs/consoles.md`, "The session's end", part 4).

## 2. Pages

- `routes::ROUTES` is the one route table and `routes::draw` the one
  dispatch: a new page joins `ROUTES`, `routes::Page` and `draw`. A page is
  an `async fn` returning `routes::Drawn`, its title and body, drawn in the
  shell by `shell::framed` or bare for the sign-in and signed-out pages. A
  route parameter is read with `routes::params()`.
- Every route takes `GET`, `HEAD` and `POST`: a post renders the page it was
  posted to, with `crate::posted::posted()` as the form. A page's model is
  one read (`read_…`) that answers the post, reads the CDR, redirects and
  sets the status, and the page draws it with `pages::common::drawn`.
- A form's default action is a hidden `default-intent`; a submit button's
  `name="intent"` wins (`Posted::intent`), whatever the order of the fields.
- A success redirects (`crate::flash::redirect_with`, a toast in a flash
  cookie); a refusal renders on the page posted to, the fields as typed.
- A page that docks its decision bar calls `shell::dock` before it returns,
  and the shell draws the bar. Discard leaves with `formnovalidate`
  (FerroUI's `Decision::leaves`), so a docked form may mark a field
  `required`; the save checks it again.
- A form never holds another form (the parser drops it); a second action in a
  form is a submit with `formaction`.
- A table row is one link (FerroUI R9); another control in the row sits in
  `.fui-table-actions` or the row menu.
- The top bar's search posts `q=` to `shell::FIND_ANSWER_ROUTE` and answers
  the JSON FerroUI's `search` module reads (FerroUI `docs/consoles.md`, "The
  top bar's search"); with no script the search is a form to `/find`.
- A page writes its own markup around FerroUI's parts with the
  `crate::markup` shorthands, every text escaped through `markup::t`.
- The viewer's own compositions live in `src/components.rs`: the code
  editor with its diagnostics, the template source upload, the subject
  reveal. Each works with no script: Reveal is a link, and the text areas
  post with their forms.

## 3. URL state and patient data

- Filter and search state lives in the URL (a `GET` form); paging of a
  listing that names no patient may too.
- **No patient identifier and no position in a listing that can show patient
  data goes in a URL** (FerroUI's mandate, its `console-ui.md` §0): those
  listings page through `crate::paging`, a session cookie per listing.

## 4. Theme, preferences and styles

- The theme and every other display preference is a cookie of its own
  (`crate::prefs` over FerroUI's `Preferences`), posted from the user menu and
  Your preferences and read when a page is drawn; no browser storage.
- FerroUI's stylesheet, its FerroEHR theme and its scripts are served from
  this origin under `ferrohealth_ui::serve::PREFIX` (`src/server.rs`). The
  viewer adds no stylesheet and no script of its own.

## 5. Testing and gates

- FerroUI's own gates (its `console-ui.md` §10) hold the library; the
  design checks of the viewer's mockup (`design/COMPONENTS.md` § Checks every
  console runs, `design/tools/check.sh`) run in FerroUI. The viewer runs the
  gates below over its own code.
- Business logic lives out of the page functions in plain types with unit
  tests.
- `tests/it/ssr_app.rs` drives every screen through the real service in
  process, over a CDR replayed from `tests/fixtures/cdr/recording.json`.
- **E2E is a merge gate**: `thirtyfour` driving headless Chromium against the
  composed stack (`scripts/ui-e2e.sh`), journeys in the `e2e_*` modules,
  skip-with-reason when `UI_E2E_BASE_URL` is unset. Every journey fails on a
  SEVERE browser-console entry. Explicit waits, never `sleep`; a journey
  holds the harness's bounded `common::Element`
  (`scripts/checks/e2e-waits.sh`), and a step that needs a module waits for
  FerroUI's `html[data-fui-script]` (`Harness::wait_scripts`). CI runs the
  battery at 1440 px and at 390 px (`UI_E2E_VIEWPORT=phone`, device
  emulation). `e2e_no_script` drives the pages with FerroUI's scripts
  refused, `e2e_navigation` holds the document loads and the 320 px reflow
  with the scripts loaded and refused, and `e2e_keyboard` drives the
  composition-level keyboard paths (#2793).
- Gates: `cargo clippy -p ferroehr-viewer --all-targets -- -D warnings`;
  `cargo nextest run -p ferroehr-viewer`; `cargo fmt`.
- **The `ui-screenshot-guard` CI job**: a PR that touches
  `app/ferroehr-viewer/src/` commits refreshed captures under
  `website/book/src/viewer/img/` (`UI_E2E_DOCS_SHOTS=1 scripts/ui-e2e.sh`) or
  carries the `no-ui-visual-change` label (zero visual effect only). It reads
  only the PR's own commits, from the merge base
  (`scripts/checks/ui-screenshot.sh`, #4117). Applying the label raises a
  fresh run (#2777). The same pass captures every route at
  1440 and 390 px, light and dark, as the `ui-route-shots` artifact (#3910).

## 6. Strings and locales

The catalog rule is FerroUI's (its `console-ui.md` §13): no string the
viewer draws lives outside a catalog, and a language joins as a catalog file
with no code change.

- The viewer's catalogs are
  `app/ferroehr-viewer/locales/<part>/<tag>/<part>.ftl`, one for each part
  that looks its words up, embedded by `fluent_templates::static_loader!`:
  `shell` holds the words of the shell and of more than one part
  (`crate::locale::Msg`), each page group has its own (`overview`,
  `records`, `definitions`, `queries`, `console`, `signin`, each with its
  message type in `crate::locale::<part>`), and `server` holds the words of
  the server's own answers. A message goes in the catalog of the one group
  that draws it, else the shell's; a message no code draws is removed. A
  test holds every part's catalog directories to its full message set in
  the shell's languages, and a throwaway catalog under `tests/locales/`
  shows the words change with no code change.
- The server chooses the locale once per request (`crate::locale::negotiated`,
  read through `crate::locale::current`): the best language of
  `Accept-Language` the catalog holds, else `[ui] locale`. The document
  writes it as `<html lang>`, and every FerroUI part is drawn with the same
  `Locale`, so the viewer's words and FerroUI's switch together.
- `scripts/checks/stray-literals.sh app/ferroehr-viewer/src` runs in CI
  against `scripts/checks/stray-literals.baseline`: a file may hold no more
  stray literals than its count. A change that moves strings into the
  catalog passes with a note and leaves the baseline alone; the
  `Ratchet tighten` workflow lowers the counts on `main`. A literal no reader sees (an RM
  type name, a header token, a cookie's attributes) is allowed by pattern in
  `scripts/checks/stray-literals.allow`, with the reason.
