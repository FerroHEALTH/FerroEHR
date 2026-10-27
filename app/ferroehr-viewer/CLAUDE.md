# `ferroehr-viewer`: the viewer (a FerroUI console over ITS-REST)

A standalone Leptos 0.8 app in islands mode, built by cargo-leptos (the server
binary and the WebAssembly islands, the two halves chosen by compilation
target, never by Cargo feature) with its own OCI image. It is a REST client
of the CDR; no openEHR spec governs the viewer (our own design); the wire it
consumes is spec-bound (`docs/specs/openehr/ITS-REST/` and the served
`openapi.json`).

## The binding mandates

1. **Rust only, zero authored JavaScript.** The wasm-bindgen bootstrap is
   generated, never touched; the stylesheet is FerroUI's, served by the
   viewer; there is no Node and no Tailwind.
2. **The CDR is reached only over ITS-REST.** Never depend on `app/ferroehr`
   or `app/ferroehr-rest`; allowed deps: `crates/openehr-*` and the network.
   The two source files shared with `app/ferroehr` are compiled here through
   `#[path]` modules and hold constants only: `app/ferroehr/src/manufacturer.rs`
   (`crate::manufacturer`) and `app/ferroehr/src/served_vocabulary.rs`
   (`crate::served_vocabulary`, the deployment gap codes and the OpenAPI family
   slugs).
3. **Every route is a public HTTP endpoint.** A page checks the session
   itself (the login guard in `src/server.rs`); a post is checked for its
   `Origin`; a `#[server]` fn left for an island checks the session too and
   is declared with `crate::session_client::SessionAwareClient`.

## How a page works (FerroUI's interaction contract)

- **The server draws every page whole** (`SsrMode::Async`); the browser
  hydrates only the islands (`src/islands.rs` and FerroUI's own). Every link
  is a link and every action a form, so each page works with no WebAssembly:
  native elements first (`<dialog>` with `command`/`commandfor`, `popover`,
  `<details>`), then Rust behaviour in an island, never hand-written JS.
- **A post renders the page it was posted to.** Every route takes `GET` and
  `POST`; `crate::posted::posted()` is the form the page answers. A form's
  default action rides as `default-intent` and the submitter's own `intent`
  wins (`Posted::intent`). A success redirects with a toast waiting in a flash
  cookie (`crate::flash::redirect_with`); a redirect is never overwritten by a
  later `set_status` (`crate::request::set_status`).
- **A docked decision bar** (`pages::common::dock_editor`, FerroUI's dock) is
  set by the page and drawn by the shell. A page that docks inside an awaited
  future holds `crate::shell::promise_dock()` in that future, or the shell may
  draw before the bar is set. Discard leaves without the form's checks
  (`Decision::leaves`, drawn as `formnovalidate`), so a docked form may mark
  its required fields; the save checks them again.
- **A table row is one link** (FerroUI R9): the first cell's link covers the
  row; any other control in the row sits in `.fui-table-actions` or the row
  menu.
- **A form never holds another form**: the HTML parser drops a nested one.
  The results pane (E7) rides the page's own form: Table and Chart are its
  submits, and each export is a submit with `formaction`.
- **The request state is shared** (`crate::request::Visit`, read once by the
  server and provided through `SharedValue`): a view reads the path, query,
  display cookies, who is signed in and the settings through `crate::request`
  on both halves, and never the credential or the session cookie.
- **Session listings keep their position in a cookie** (`crate::paging`,
  FerroUI `leptos-ui.md` §0): the EHRs, an EHR's compositions and
  contributions, the audit log. Filters stay in the URL.
- **Preferences are cookies** (`crate::prefs`, FerroUI's `Preferences`), each
  its own, read when a page is drawn. The theme is set in the user menu and on
  Your preferences, never in the top bar.
- **Words come from the catalog** (`locales/en/ferroehr-viewer.ftl` through
  `crate::locale::Msg`); `scripts/checks/stray-literals.sh` fails a literal a
  reader sees. Numbers format through FerroUI's formatter.

## The components are FerroUI's

`ferrohealth-ui`, pinned exactly in the root `[workspace.dependencies]`, read
at its tag (`docs/consoles.md`, `COMPONENTS.md`, `crates/ferrohealth-ui/CLAUDE.md`,
`design/consoles/ferroehr/` for this console's mockups). The shell, the
navigation, tables and pagers, toasts, alerts, dialogs, menus, buttons,
fields, the code editor and panels, trees, charts, the conditions of the query
builder and the stylesheet are all FerroUI's. A component the viewer needs
that FerroUI lacks is filed on FerroUI and added there, never built here.

## One reader per claim

- No two viewer surfaces read the same claim from two endpoints; where the CDR
  serves one fact twice, one page reads it and the others link there (the top
  bar's pill reads the status document; Server's Health reads the readiness
  indicators; the redacted configuration lives on Configuration; the
  conformance manifest on About).
- The operational template is read once per template page, and its panes are
  distilled from that one document.
- Directory history is a window, never a walk: ITS-REST exposes no
  `VERSIONED_FOLDER` revision history (AMB-24, upstream report #1490), so
  `list_directory_versions` synthesizes the uid list and reads the newest
  versions concurrently; Older versions widens the window (`?versions=`).
- The current `EHR_STATUS` is read once per EHR page and shared by the header
  and the Status tab.
- The compositions filters are AQL assembled in one place
  (`bff::ehr_detail::composition_filter`): fixed fragments chosen by which
  filters are filled, every value a binding. Never concatenate a value into
  AQL.
- Every scope the viewer explains is parsed by `openehr_sdt::smart_scopes`,
  the grammar the CDR enforces with; never a second parser.
- Every BFF fan-out is bounded by `cdr::FANOUT_CONCURRENCY`.
- Optional CDR surfaces are probe-and-hide: the management tabs on
  `GET /management/info`, Subscriptions on `GET /admin/event_subscription`; a
  `404` hides the entry, any other answer counts as present.

## No viewer-local domain state

The viewer stores nothing of its own: no database, no file beside the
binary. Its state is the sealed session cookie and the display cookies above.
Every fact a page shows is read from the CDR. Grouping derives from the wire
(stored-query namespaces, `crate::query_namespace`), never from a local store.

## Feedback

A write reports its outcome: a success as a toast after the redirect, a
refusal on the page that was posted, beside the fields that caused it, with
the CDR's own words (`pages::common::refused`). A read that fails says so
where its data would be; a `404` that means "absent" is an empty state, not an
error.

## Accepted build warning (adjudicated, #2697)

macOS/aarch64 bin links print Apple ld's `__eh_frame section too large (max
16MB) to encode dwarf unwind offsets in compact unwind table, performance of
exception handling might be affected`. Accepted with record: the shipped
viewer is a Linux ELF image, and on local macOS builds the cost is slower
DWARF unwinding on the already-exceptional panic path. Do not re-file it, and
do not silence `linker_messages`.

## Gates

`/ui-gates`: clippy on the host (the server half, `--all-targets`) and wasm32
(the browser half, `--lib`), `cargo nextest run -p ferroehr-viewer` (the
in-process `ssr_app` suite replays a recorded CDR, `tests/fixtures/cdr/`,
which `scripts/viewer-record.sh` records again and nothing edits by hand),
leptosfmt (src and tests) then cargo fmt, `bash scripts/cargo-leptos.sh build`
(the wrapper: it freezes `Cargo.lock` and builds with `--split`, which
FerroUI's lazy islands need, FerroUI #331). The E2E journeys (the `e2e_*`
modules under `tests/it/`, skip-with-reason via `UI_E2E_BASE_URL`) gate merges
in CI at 1440 px and at 390 px (`UI_E2E_VIEWPORT=phone`); a UI-visual change
re-captures the book's screenshots (`UI_E2E_DOCS_SHOTS`, `ui-screenshot-guard`).
