# `ferroehr-viewer`: the viewer (a FerroUI console over ITS-REST)

A standalone axum server that draws every page as HTML with FerroUI 0.6's
components and serves FerroUI's stylesheet, theme and scripts compiled into
it, one binary with its own OCI image. It is a REST client of the CDR; no
openEHR spec governs the viewer (our own design); the wire it consumes is
spec-bound (`docs/specs/openehr/ITS-REST/` and the served `openapi.json`).

## The binding mandates

1. **Rust only, zero authored JavaScript.** The only script is FerroUI's
   loader and its modules, served by the viewer; the stylesheet is FerroUI's;
   there is no Node, no WebAssembly and no Tailwind. No page writes a
   `style` attribute or a `<style>` element, so the policy is
   `script-src 'self'; style-src 'self'` (`server::POLICY`).
2. **The CDR is reached only over ITS-REST.** Never depend on `app/ferroehr`
   or `app/ferroehr-rest`; allowed deps: `crates/openehr-*` and the network.
   The two source files shared with `app/ferroehr` are compiled here through
   `#[path]` modules and hold constants only: `app/ferroehr/src/manufacturer.rs`
   (`crate::manufacturer`) and `app/ferroehr/src/served_vocabulary.rs`
   (`crate::served_vocabulary`, the deployment gap codes and the OpenAPI family
   slugs).
3. **Every route is a public HTTP endpoint.** A page checks the session
   itself (the login guard in `src/server.rs`); a request that is not a safe
   method passes `ferrohealth_server::same_origin` first; every read of the
   CDR calls `crate::session::require_session`.

## The server half is the family's

`ferrohealth-server` (`console`, `reqwest`) carries the process plumbing: the
console headers (`console::headers`, with the viewer's own `POLICY` set over
the policy it writes until it writes one for a server-rendered console),
`same_origin`, the stack with the health routes (`server::app`),
`cli::Common` and the healthcheck (`src/cli.rs`), the bounded drain
(`serve::run` over `[server]`), and the outbound TLS of the CDR and issuer
clients (`tls_client`, `[cdr.tls]` and `[auth.oidc.tls]`). A habit the crate
lacks is a FerroSYS issue, never a local copy.

## How a page works (FerroUI's interaction contract)

- **The server draws every page whole.** `routes::ROUTES` is the route
  table and `routes::draw` the dispatch; the page handler in `src/server.rs`
  runs it inside the request's `crate::scope::Scope` and wraps the page in
  the shell (`shell::framed`) and the document (`app::page_document`). A
  link and a `GET` form load a new document; FerroUI's modules add
  behaviour on top of what the server drew (`data-fui`), and every page
  works with them refused: native elements first (`<dialog>` with
  `command`/`commandfor`, `popover`, `<details>`), then a link or a form
  behind every action.
- **A page reads one model** (`read_…`, drawn with `common::drawn`): it
  answers the post, reads the CDR, redirects and sets the status. A route
  parameter is read with `routes::params()`.
- **A post renders the page it was posted to.** Every route takes `GET`,
  `HEAD` and `POST`; `crate::posted::posted()` is the form the page answers.
  A form's default action rides as `default-intent` and the submitter's own
  `intent` wins (`Posted::intent`). A success redirects with a toast waiting
  in a flash cookie (`crate::flash::redirect_with`); a redirect is never
  overwritten by a later `set_status` (`crate::request::set_status`).
- **A dead session ends the page:** a refused session or a CDR `401` is
  noted in the scope (`session_client::note_if_over`) and the page answers
  with the sign-in page.
- **A docked decision bar** (`pages::common::dock_editor`, FerroUI's dock) is
  set by the page with `shell::dock` and drawn by the shell. Discard leaves
  without the form's checks (`Decision::leaves`, drawn as `formnovalidate`),
  so a docked form may mark its required fields; the save checks them again.
- **A table row is one link** (FerroUI R9): the first cell's link covers the
  row; any other control in the row sits in `.fui-table-actions` or the row
  menu.
- **A form never holds another form**: the HTML parser drops a nested one.
  The results pane (E7) rides the page's own form: Table and Chart are its
  submits, and each export is a submit with `formaction`.
- **The request is read through the scope**: `crate::request` (path, query,
  display cookies, who is signed in, settings, the instant the page is
  drawn at), never the credential or the session cookie in drawn HTML.
- **Session listings keep their position in a cookie** (`crate::paging`,
  FerroUI `console-ui.md` §0): the EHRs, an EHR's compositions and
  contributions, the audit log. Filters stay in the URL.
- **Preferences are cookies** (`crate::prefs`, FerroUI's `Preferences`), each
  its own, read when a page is drawn. The theme is set in the user menu and on
  Your preferences, never in the top bar.
- **Words come from the catalogs** (`locales/<part>/en/<part>.ftl`, the
  shell's through `crate::locale::Msg` and each page group's through its
  own type in `crate::locale::<part>`, `.claude/rules/console-ui.md` §6);
  `scripts/checks/stray-literals.sh` fails a literal a reader sees. Numbers
  format through FerroUI's formatter. A page's own markup is written with
  `crate::markup`, every text escaped.

## The components are FerroUI's

`ferrohealth-ui`, pinned exactly in the root `[workspace.dependencies]`, read
at its tag (`docs/consoles.md`, `docs/migration-0.6.md`,
`.claude/rules/console-ui.md`, `design/COMPONENTS.md`,
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
- Every scope the viewer explains is parsed by `ferrohealth_smart::openehr::smart_scopes`,
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

`/ui-gates`: `cargo clippy -p ferroehr-viewer --all-targets -- -D warnings`,
`cargo nextest run -p ferroehr-viewer` (the in-process `ssr_app` suite
replays a recorded CDR, `tests/fixtures/cdr/`, which
`scripts/viewer-record.sh` records again and nothing edits by hand) and
`cargo fmt`. The E2E journeys (the `e2e_*` modules under `tests/it/`,
skip-with-reason via `UI_E2E_BASE_URL`) gate merges in CI at 1440 px and at
390 px (`UI_E2E_VIEWPORT=phone`), each step that needs a module waiting for
FerroUI's `html[data-fui-script]`; `e2e_navigation` holds the document
loads and the 320 px reflow, and `e2e_no_script` every page with FerroUI's
scripts refused. A UI-visual change re-captures the book's screenshots
(`UI_E2E_DOCS_SHOTS`, `ui-screenshot-guard`).
