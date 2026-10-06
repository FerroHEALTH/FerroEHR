# `ferroehr-viewer` — the Leptos viewer (BFF over ITS-REST)

A standalone full-stack Leptos 0.8 app (cargo-leptos: `ssr` server binary +
`hydrate` WASM client) and its own OCI image. It is a **REST client** of the
CDR — no openEHR spec governs the viewer (our own design / product
extension); the wire it consumes IS spec-bound (`docs/specs/openehr/ITS-REST/`
+ the served `openapi.json`).

## The three binding mandates (owner, 2026-07-13)

1. **Rust only — zero authored JavaScript** (the wasm-bindgen bootstrap is
   generated, never touched; styling is Tailwind v4 standalone, no Node).
2. **The CDR is reached ONLY over ITS-REST.** Never depend on `app/ferroehr`
   or `app/ferroehr-rest`; allowed deps: `crates/openehr-*` + the network.
   The one source file shared with `app/ferroehr` is
   `app/ferroehr/src/manufacturer.rs`, compiled here through a `#[path]`
   module (`crate::manufacturer`): it holds constants only and needs nothing
   but `serde`, so the footer and the `/system` manufacturer card name the
   manufacturer without a request and without a Cargo dependency.
3. **Every `#[server]` fn is a publicly reachable HTTP endpoint** — each one
   enforces the viewer's own session auth itself; "only my UI calls this"
   is never assumed.

## Discipline

- Rules file: `.claude/rules/leptos-ui.md` (hydration hard rules, `<For>`
  keys, forms, server fns); Leptos questions → `/leptos-lookup` (the book is
  the oracle, never memory).
- Business logic (query-builder AST lowering, criteria validation) lives in
  component-free plain-Rust modules with ordinary unit tests; components
  stay thin.
- Pins (root `[workspace.dependencies]`): `leptos`/`leptos_axum`/`leptos_meta`/
  `leptos_router` 0.8.x · `thaw` at a pinned **git rev of main** (the crates.io
  `0.5.0-beta` lacks `#![recursion_limit = "256"]` and fails plain,
  non-`erase_components` codegen on rustc 1.96; re-pin to crates.io at 0.5
  stable) · `leptos-use` 0.19 · `leptos-struct-table` 0.19 · `leptos-chartistry`
  0.2.3 · `leptos_icons` 0.7.1.
- **Views are built in `.into_any()`-erased sections** (rules §1): plain cargo
  builds have no `erase_components`, and monolithic thaw view trees blow rustc's
  layout-recursion depth in `cargo test` codegen.
- **Every listing table comes from ONE kit — `components::data_table`**: the
  table shell, the loading skeleton (`table_skeleton`; never re-declare a
  per-screen copy), the viewer-wide `PAGE_SIZE`, and the pagination footer
  (`table_footer`) whose page + window size are URL state (`?page=`/`?size=`,
  read in SETUP via `paging_from_url` — never inside a `Suspend`, so paging
  re-renders the window without refetching). The footer's row math
  (`page_window`/`page_rows`) is pure and unit-tested; the AQL-windowed tables
  (EHRs, compositions, results) keep their offset controls until the wire
  reports a total.
- **Every AQL result set is charted by ONE kit — `components::results_chart`**
  (both query screens' results panes call it). What the chart shows is decided by
  the component-free, unit-tested `chart_model`: one series per mostly-numeric
  column, every ISO-8601 column offered as a real time axis with the row order as
  the fallback, and the tick-label granularity. Keep that derivation pure (it is
  what makes the chart hydration-safe) and keep the view thin: a legend-hidden
  series is drawn as `f64::NAN` — chartistry's missing-data marker — rather than
  by rebuilding the chart, and each line pins its palette colour by index so
  hiding one never recolours the others.
- **Every scope string the viewer EXPLAINS is read by ONE grammar and rendered
  by ONE kit.**
  The parse is `openehr_sdt::smart_scopes` — the same master08 module the
  CDR's scope gate enforces with, so the viewer's explanation can never drift
  from the server's behaviour; NEVER write a second scope parser here. It is
  reachable on BOTH targets because the crate is taken
  `default-features = false` (the grammar is a dependency-free island; the heavy
  ITS surfaces ride `openehr-its/full`, which the `ssr` feature adds back) — the
  previewer is a pure client-side function, not a server round-trip. The
  presentation model is the component-free, unit-tested `crate::scopes` (grants,
  chips, the master08 diagnosis for a rejected scope, the policy-source copy);
  `components::scope_grants` is the only view that draws it, used by the access
  drawer for the session's own scopes and by the previewer field. The drawer
  renders ONLY identity facts the session actually carries (identity + method +
  scopes today) — no invented claims, and capability ≠ authorization is stated
  on the surface. `/system`'s SMART card still lists the discovery document's
  `scopes_supported` as plain chips: that is an advertisement of supported
  strings, not a grant this session holds — if it ever explains them, it uses
  this kit and never a second parser.
- **Every terminology lookup goes through ONE reader — `crate::terminology`**: the
  six session-guarded `#[server]` fns over the CDR's terminology surface, plus the
  pure, unit-tested flatteners their answers pass through (`TermRow::rubric` is the
  single `code — text` spelling). Both consumers read it — the `/terminology`
  browser screen and the query builder's coded-criterion picker — so a term looks
  the same wherever it appears. The surface is CONFIG-GATED on the CDR side
  (`[terminology] api_enabled`, off by default) and answers `404` as if unmounted,
  which is the same `404` an unknown terminology/code/value set produces: every
  read returns `Ok(None)` for it and the screen renders the absence it means
  (the browser's whole-screen disabled card; a silently optionless datalist in the
  builder). Never add a second terminology client.
- **Every BFF fan-out is bounded by ONE constant — `cdr::FANOUT_CONCURRENCY`**:
  a `#[server]` fn that issues N CDR requests for one screen runs them
  `futures::stream::iter(…).buffered(FANOUT_CONCURRENCY)`, never a serial await
  chain and never an unbounded burst (the dashboard namespace tiles, `/system`'s
  repository-usage counts, the directory history window). `try_collect` keeps a
  serial loop's short circuit where the caller had one. Never write a second
  concurrency literal.
- **Every events-per-day chart comes from ONE kit — `components::activity_chart`
  over `crate::activity::bucket_by_day`**: the dashboard's commit trend and an
  EHR's contribution timeline are the same chart, the day-bucketing is a pure
  unit-tested function (no clock, no locale — that is what keeps the chart
  hydration-stable), and charts are `leptos-chartistry` (pure Rust + SVG); a
  JS charting binding is banned by the no-JavaScript mandate.
- **Every template example pane is controlled by ONE kit —
  `components::example_controls` over `crate::example_options`** (#2894):
  format, `detail_level` and `type` are three segmented controls whose
  signals feed the pane's resource source, the query string is the pure
  unit-tested `example_query`, and every segmented control anywhere in the
  viewer draws through `format_view::segment_button`. Never build a second
  example-options control or query builder.

## Every `#[server]` fn carries the session backstop (#3044)

- **A server function is declared `#[server(client =
  crate::session_client::SessionAwareClient)]` — with no exceptions.** That
  transport wrapper is what watches every answer for
  `ViewerError::Unauthenticated` / `CdrUnauthorized` (server_fn renders an
  application error as a `500` carrying the encoded error, so the detection
  reads the BODY, not the status) and bumps the counter the shell turns into
  the signed-out transition. A bare `#[server]` is a hole in that net; the
  unit test `session_client::tests::
  every_server_function_in_this_crate_is_declared_with_the_session_aware_client`
  fails on one.
- **The signed-out transition has ONE destination —
  `session_client::signed_out_url`** (`/login?expired=1&next=…`), used by both
  the shell's session gate and its backstop Effect. The login screen renders
  the "your session ended" notice from `?expired=1` alone, so the state is a
  URL, not a signal.

## Error feedback: toast vs inline (one rule, 2026-07-25)

- **A mutation toasts on success AND on failure.** Every action that writes to
  the CDR (create / commit / update / save / upload / delete) reports both
  outcomes as a toast. The failure toast carries the **actionable** copy —
  name the object, name what went wrong (the CDR's own diagnostic verbatim),
  name the next action: `feedback::write_failure_copy` for writes,
  `admin::delete_failure_copy` for the destructive admin ops.
- **A detailed inline `MessageBar` may stay BESIDE the failure toast** where
  the diagnostic is worth reading line by line (template-upload validation, a
  rejected composition body, the directory conflict banner) — the toast is
  the notification, the inline bar is the detail. Never inline-only: a
  transient success toast paired with a silent failure below the fold reads
  as "nothing happened".
- **Pure reads render inline errors only** — `notice::inline_error` in
  the section whose data failed (a screen says so where the data would be),
  and never a toast. A first-class empty/absent state (`Ok(None)` from a
  `404`) is not an error at all. `components::notice` is the ONE home of the
  inline feedback views: the read-failure bar, the alert note beside a
  control, the deleted/unknown section notices, the CDR's verbatim diagnostic
  under a refused form, and the write-failure message bar.

## No viewer-local domain state (owner ruling, 2026-07-25)

- **The viewer stores NOTHING of its own** — no database, no JSON store beside
  the binary, no state directory. Its only state is the sealed session cookie. Every fact a screen shows is read from the CDR over ITS-REST, so it is
  visible to other clients, survives a restart, is covered by the CDR's backups,
  and is identical across replicas. The two former stores (`groups.rs` →
  `viewer-groups.json`, `folder_templates.rs` →
  `viewer-folder-templates.json`) were deleted for exactly those reasons —
  **do not reintroduce a local store in any form** (file, embedded DB, cookie
  payload); if a grouping or preset is worth keeping, find it in the wire
  contract or drop the feature.
- **Grouping derives from the wire, not from us.** Stored-query grouping is the
  namespace of the qualified name (`crate::query_namespace`, which cites
  ITS-REST `specifications/docs/query/Qualified_query_name.md`); the directory
  create flow commits an empty root and the tree editor builds structure. Any
  future "grouping"/"favourites"/"preset" feature must be derived from CDR data
  the same way, or not exist.

## One reader per claim (owner adjudication, 2026-07-25)

- **No two viewer surfaces may read the same claim from two endpoints.** Where
  the CDR exposes one fact on more than one endpoint, the viewer picks ONE
  reader and every other screen cross-links to it. Live cases: the topbar pill
  reads the status document (`/ferroehr/rest/status` — API up + version) while
  the operations panel's health card reads `/health/readiness` (dependency
  indicators), and the screen states the split; the redacted effective
  configuration is served identically by `/management/env` and `/admin/config`,
  so the ONE viewer lives on `/system` (the API base URL is always configured;
  the management surface may sit on an unreachable internal port) and
  `/operations` links to it. On `/system` the status document is the reader for
  the product + openEHR-REST versions, so the conformance-manifest card shows
  only what the System API alone knows (product identity, claimed profile,
  mounted API groups) and says where the versions are. On the composition
  viewer the split is: document CONTENT ← the COMPOSITION resource (the only
  format-negotiating one), commit history ← the revision history, the VERSION's
  envelope facts (lifecycle state, preceding version, contribution, signature)
  ← the direct VERSIONED_COMPOSITION version read.
- **The operational template is read ONCE per template-detail render**:
  `pages::template_detail::fetch_template_detail` fetches the OPT and distils
  all three panes from that one document (source, identity card, WT path
  catalog) through the pure `template_detail_from_opt`; every pane takes the
  same handle. The Query Builder's `fetch_template_catalog` rides the same
  pipeline and returns the catalog alone, so its wire payload stays the tree.
  Never add a second GET of `definition/template/adl1.4/{id}`.
- **Directory history is a WINDOW, never a walk**: ITS-REST exposes no
  VERSIONED_FOLDER revision history (register AMB-24, upstream report #1490),
  so `list_directory_versions` synthesizes the uid list and reads the newest
  `PAGE_SIZE` of it concurrently; the rest is reached by the panel's explicit
  "load older", one page per click. There is deliberately no "load all".
- **EHR_STATUS split**: the *current* status document is read ONCE per
  EHR-detail screen — `pages::ehr_detail::status::status_feed`, created in the
  page's setup and shared by its two consumers: the header's identity strip
  (subject + the queryable/modifiable badges) and the Status tab (the same
  badges, the document pane, and the edit form's merge base). Never add a
  second GET for any of those facts; take the handle. The read is deliberately
  NOT tab-gated, because the header renders on every tab. The Status-history
  tab reads only the VERSIONED_EHR_STATUS family — content by
  `ehr_status/{version_uid}`, envelope facts by the VERSION read (the
  composition-viewer split). The edit form replaces exactly
  `is_queryable`/`is_modifiable`/`other_details` and re-sends everything else
  verbatim — never a re-model of the served document.
- **The compositions tab's filters are AQL, assembled in ONE place**:
  `pages::ehr_detail::composition_filter` is the component-free, unit-tested
  module that turns the URL's `?template=`/`?from=`/`?to=`/`?composer=` into a
  statement plus its `query_parameters`. The statement text comes from
  compile-time fragments chosen by which filters are FILLED; every operator
  value is a BINDING. Never concatenate a filter value into AQL, and never let
  the text and the bindings drift apart — the CDR answers `400 unbound query
  parameter` for a named parameter it was not given.
- **CONTRIBUTION authoring has ONE writer and no readers of its own.** The EHR
  detail's Commit tab (`pages::ehr_detail::commit`) is the viewer's only caller
  of `POST /ehr/{ehr_id}/contribution` — the openEHR-native atomic change set —
  and it re-uses the existing readers for everything it shows: the template list,
  the EHR's composition list, the composition body, and the current `EHR_STATUS`.
  It adds no viewer either: a committed contribution opens in the Contributions
  tab. Its staging list is component state, never a store (§No viewer-local
  domain state), and the body it posts is built by the component-free,
  unit-tested `commit::staged` — never inline in a view.
- **Two windows of ONE endpoint are not two readers.** The contributions tab
  reads `GET /ehr/{id}/contribution` twice — a page for the list, a wider window
  for the activity timeline — because those are different claims; what the rule
  forbids is reading one claim from two different endpoints.
- **Optional CDR surfaces are probe-and-hide.** An affordance for a surface the
  CDR may not serve is gated on a probe (`crate::admin` for the admin group via
  the System API manifest, `crate::management` for the management surface via
  `GET /management/info`): a `404` hides the affordance/nav entry entirely, and
  every other answer counts as present — capability is not authorization, so a
  `401`/`403` refusal surfaces as actionable copy on the screen that asked.

## Accepted build warning (adjudicated, #2697)

macOS/aarch64 bin links print Apple ld's `__eh_frame section too large (max
16MB) to encode dwarf unwind offsets in compact unwind table, performance of
exception handling might be affected`. Accepted with record: the shipped
viewer is a Linux ELF image (Apple's compact-unwind machinery never applies
to production), and on local macOS builds the cost is slower DWARF unwinding
on the already-exceptional panic path — the `panic = "unwind"` contract needs
unwinding to WORK, not to be fast. Do not re-file it, and do not silence
`linker_messages` (it would hide genuinely new linker findings).

## Gates

`/ui-gates`: clippy on **native (`--features ssr`) and wasm32 (hydrate)**
targets, `cargo nextest run -p ferroehr-viewer --features ssr` (the
featureless crate ships nowhere and skips every ssr-gated test), leptosfmt
(src + tests) + cargo fmt, `bash scripts/cargo-leptos.sh build` (the wrapper,
never a bare `cargo leptos` — it rewrites `Cargo.lock`, #2877); E2E journeys (the `e2e_*`
modules under `tests/it/`, skip-with-reason via `UI_E2E_BASE_URL`)
merge-gate in CI; a UI-visual change re-captures the `website/book`
screenshots (`ui-screenshot-guard`).
