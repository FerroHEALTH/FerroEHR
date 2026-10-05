// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared table kit.
//!
//! One styled `<table>` used by every listing screen (EHRs, compositions,
//! stored queries, results) so tables stop being hand-rolled per page, the
//! loading skeleton every listing falls back to, and the pagination footer the
//! paged tables share. Headers are NAMED — raw AQL column indexes (`#0`…) must
//! never reach a header cell.
//!
//! Paging discipline: a table's page and window size live in the URL
//! (`?page=`/`?size=`), never in a private signal — the address bar is
//! shareable, the browser's back/forward walk the pages, and the footer's
//! plain links work before the WASM bundle has loaded. Both parameters are
//! user input, so both are parsed defensively and clamped
//! ([`paging_from_url`], [`page_window`]), and every offset is saturating.

use leptos::prelude::*;
use leptos_router::params::ParamsMap;

use crate::components::field::BTN_SECONDARY;

/// Class set for a body row (hover tint; hairline separators).
pub const ROW: &str = "border-b border-edge last:border-0 hover:bg-sunken/60";

/// Class set for a body cell.
pub const CELL: &str = "px-3 py-2 align-top";

/// Class set for a monospace body cell (ids, paths, AQL).
pub const CELL_MONO: &str = "px-3 py-2 align-top font-mono text-xs";

/// Rows per page across the viewer's tables.
///
/// The AQL fetch window (`fetch`/`_count` on the wire) and the default window
/// of the tables that page rows already in hand, so every listing pages in the
/// same size.
pub const PAGE_SIZE: u32 = 25;

/// The smallest window a `?size=` may ask for: the parameter is user input and
/// a zero would divide by zero in [`page_window`].
const MIN_PAGE_SIZE: u32 = 1;

/// The largest window a `?size=` may ask for — a hand-typed `?size=100000`
/// must not render a hundred thousand rows into one page.
const MAX_PAGE_SIZE: u32 = 200;

/// The window sizes the footer offers as one-click choices.
const PAGE_SIZE_CHOICES: [u32; 3] = [PAGE_SIZE, 50, 100];

/// The styled table shell around pre-rendered `<tr>` rows.
///
/// Renders the card surface, the muted uppercase header row, and an
/// explicit `<tbody>` (hydration correctness: browsers insert one
/// otherwise, breaking DOM↔view correspondence). `body` is the collected
/// `<tr>` views; build cells with [`CELL`]/[`CELL_MONO`] and rows with
/// [`ROW`].
///
/// Every header cell carries `scope="col"`, so a screen reader announces the
/// column name with each body cell it reads (WAI-ARIA Authoring Practices,
/// "Table" pattern; the HTML `th` `scope` attribute) — the viewer's tables
/// are all simple column-headed grids, which is exactly the case `scope`
/// covers.
#[must_use]
pub fn table_shell(headers: &[&str], body: AnyView) -> AnyView {
    let head = headers
        .iter()
        .map(|h| {
            view! {
                <th
                    scope="col"
                    class="px-3 py-2 text-left text-xs font-semibold uppercase tracking-wide text-ink-muted"
                >
                    {h.to_string()}
                </th>
            }
        })
        .collect_view();
    view! {
        <div class="overflow-x-auto rounded-card border border-edge bg-raised shadow-card">
            <table class="w-full border-collapse text-sm text-ink">
                <thead class="border-b border-edge bg-sunken">
                    <tr>{head}</tr>
                </thead>
                <tbody>{body}</tbody>
            </table>
        </div>
    }
    .into_any()
}

/// The `<Transition>`/`<Suspense>` fallback every listing shares: three
/// skeleton bars standing in for the rows while the data loads.
///
/// ONE definition for the whole viewer — passed as the fallback itself
/// (`fallback=table_skeleton`).
#[must_use]
pub fn table_skeleton() -> impl IntoView {
    view! {
        <thaw::Skeleton>
            <thaw::SkeletonItem class="h-4 mb-2" />
            <thaw::SkeletonItem class="h-4 mb-2" />
            <thaw::SkeletonItem class="h-4" />
        </thaw::Skeleton>
    }
}

/// A paged table's state as it lives in the URL.
///
/// The zero-based page index (`?page=`) and the row-window size (`?size=`),
/// plus the rest of the query so a page link preserves the screen's other
/// parameters.
///
/// Built with [`paging_from_url`]; both signals derive from the address bar
/// alone, so the server pass and hydration agree on the rendered window.
#[derive(Debug, Clone, Copy)]
pub struct TablePaging {
    /// The zero-based page index from `?page=` (0 when absent or unparseable).
    /// Unclamped — [`page_window`] clamps it against the actual row total, so
    /// a page number past the end shows the last page instead of nothing.
    pub page: Signal<u32>,
    /// The row-window size from `?size=`, clamped to a sane range and
    /// defaulting to [`PAGE_SIZE`].
    pub size: Signal<u32>,
    /// The whole current query map, so a page link keeps every OTHER parameter
    /// on the screen instead of dropping it.
    query: Memo<ParamsMap>,
}

impl TablePaging {
    /// The href that pages a table at `base` to `page` with window `size`,
    /// carrying every other query parameter across.
    ///
    /// The default page and the default size are written as their ABSENCE, so
    /// the first page's URL is the screen's plain path. Percent-encoding is
    /// the router's own [`ParamsMap::to_query_string`] — never a hand-rolled
    /// codec.
    fn href(self, base: &str, page: u32, size: u32) -> String {
        let mut map = self.query.get();
        if page == 0 {
            drop(map.remove("page"));
        } else {
            map.replace("page", page.to_string());
        }
        if size == PAGE_SIZE {
            drop(map.remove("size"));
        } else {
            map.replace("size", size.to_string());
        }
        format!("{base}{}", map.to_query_string())
    }
}

/// Read a table's paging state from the URL.
///
/// Call this in SETUP, never inside a `Suspend` closure: the returned signals
/// are read where the rows are rendered, so turning the page re-renders the
/// row window without re-running the suspense that fetched the rows — no
/// refetch, and no re-created resources.
#[must_use]
pub fn paging_from_url() -> TablePaging {
    let query = leptos_router::hooks::use_query_map();
    let page = Signal::derive(move || {
        query
            .with(|q| q.get("page"))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(0)
    });
    let size = Signal::derive(move || {
        query
            .with(|q| q.get("size"))
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(PAGE_SIZE)
            .clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE)
    });
    TablePaging { page, size, query }
}

/// The row window a paged table renders: the clamped page, its size, the row
/// total it was computed against, and the half-open `[start, end)` row range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageWindow {
    /// The page actually shown — the requested page clamped to the last one
    /// that holds rows.
    pub page: u32,
    /// The window size actually applied (clamped).
    pub size: u32,
    /// The number of rows the window was computed against.
    pub total: u32,
    /// Index of the window's first row.
    pub start: u32,
    /// Index one past the window's last row.
    pub end: u32,
}

/// Compute the row window for `page` of `size` over `total` rows.
///
/// Both inputs are URL parameters, so both are tamed here: the size is clamped
/// to `1..=200`, and a page past the end clamps to the last page that holds
/// rows (deleting the last row therefore lands the reader on rows, never on a
/// blank table). All arithmetic saturates.
#[must_use]
pub fn page_window(total: u32, page: u32, size: u32) -> PageWindow {
    let size = size.clamp(MIN_PAGE_SIZE, MAX_PAGE_SIZE);
    #[expect(
        clippy::integer_division,
        reason = "a page index IS the truncating quotient: row `total-1` sits on page `(total-1)/size`"
    )]
    let last_page = total.saturating_sub(1) / size;
    let page = page.min(last_page);
    let start = page.saturating_mul(size).min(total);
    let end = start.saturating_add(size).min(total);
    PageWindow {
        page,
        size,
        total,
        start,
        end,
    }
}

/// The rows of `window` out of the full row list — the slice the table
/// renders. A window outside the list yields no rows rather than panicking.
#[must_use]
pub fn page_rows<T: Clone>(rows: &[T], window: PageWindow) -> Vec<T> {
    let start = usize::try_from(window.start).unwrap_or(usize::MAX);
    let end = usize::try_from(window.end).unwrap_or(usize::MAX);
    rows.get(start..end).map(<[T]>::to_vec).unwrap_or_default()
}

/// A row count as the fixed-size int the paging math and the footer use.
///
/// WASM is 32-bit and no shared type may carry a `usize`, so a list's
/// `.len()` converts here once, saturating rather than wrapping.
#[must_use]
pub fn row_total(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// The footer's "which rows am I looking at" line, e.g.
/// `26–50 of 137 templates`. `noun` names the rows in the plural.
#[must_use]
pub fn range_label(window: PageWindow, noun: &str) -> String {
    if window.total == 0 {
        return format!("No {noun}");
    }
    format!(
        "{}–{} of {} {noun}",
        window.start.saturating_add(1),
        window.end,
        window.total
    )
}

/// The shared pagination footer: which rows are on screen, previous/next, and
/// the page-size choices — for a table whose rows are all in hand, so the row
/// total is known.
///
/// `base` is the screen's own path (`/queries`), `noun` names the rows in the
/// plural ("stored queries"), `paging` comes from [`paging_from_url`], and
/// `total` is the (reactive) row count — reactive because a client-side filter
/// changes it. The caller renders its rows through [`page_window`] +
/// [`page_rows`] from the SAME inputs, so the footer and the rows cannot
/// disagree.
///
/// Every control is a plain link, so paging works before the WASM bundle loads
/// and the browser's history walks the pages. Server-windowed tables (an AQL
/// result set, where only the current page is in hand) keep their own offset
/// controls until the wire reports a total.
#[must_use]
pub fn table_footer(base: &str, noun: &str, paging: TablePaging, total: Signal<u32>) -> AnyView {
    let base = base.to_owned();
    let noun_label = noun.to_owned();
    let window = move || page_window(total.get(), paging.page.get(), paging.size.get());

    let range = {
        let noun = noun.to_owned();
        move || range_label(window(), &noun)
    };
    let steps = {
        let base = base.clone();
        move || {
            let current = window();
            let previous = (current.page > 0)
                .then(|| paging.href(&base, current.page.saturating_sub(1), current.size));
            let next = (current.end < current.total)
                .then(|| paging.href(&base, current.page.saturating_add(1), current.size));
            view! {
                <div class="flex items-center gap-2">
                    {paging_step(previous, Step::Previous)} {paging_step(next, Step::Next)}
                </div>
            }
            .into_any()
        }
    };
    let sizes = move || {
        let current = window();
        let choices = PAGE_SIZE_CHOICES
            .iter()
            .map(|&choice| size_choice(&base, paging, choice, current.size))
            .collect::<Vec<_>>();
        view! {
            <span class="flex items-center gap-1">
                <span>"Rows per page"</span>
                {choices}
            </span>
        }
        .into_any()
    };

    view! {
        <div
            class="mt-3 flex flex-wrap items-center justify-between gap-3 text-xs text-ink-muted"
            data-table-footer=noun_label
        >
            <span data-page="range">{range}</span>
            {steps}
            {sizes}
        </div>
    }
    .into_any()
}

/// A whole paged table over rows already in hand: the styled shell, the row
/// window, and the footer that pages it.
///
/// The window comes from the URL, so turning the page re-renders the rows
/// without re-running the suspense that fetched them, and the footer reads the
/// SAME [`page_window`] inputs as the rows, so the two cannot disagree. `key`
/// is the row's own stable, data-derived identity — never an index. `base` is
/// the screen's own path and `noun` names the rows in the plural, both for
/// [`table_footer`].
#[must_use]
pub fn paged_table<R, V>(
    rows: Vec<R>,
    paging: TablePaging,
    base: &str,
    noun: &str,
    headers: &[&str],
    key: fn(&R) -> String,
    row: impl Fn(R) -> V + Clone + Send + Sync + 'static,
) -> AnyView
where
    R: Clone + Send + Sync + 'static,
    V: IntoView + 'static,
{
    let count = row_total(rows.len());
    let total = Signal::derive(move || count);
    let body = view! {
        <For
            each=move || {
                let window = page_window(total.get(), paging.page.get(), paging.size.get());
                page_rows(&rows, window)
            }
            key=key
            children=row
        />
    }
    .into_any();
    let footer = table_footer(base, noun, paging, total);
    view! {
        {table_shell(headers, body)}
        {footer}
    }
    .into_any()
}

/// Which way a paging step moves — its label, its icon, and its E2E hook.
#[derive(Debug, Clone, Copy)]
enum Step {
    /// Towards the first page.
    Previous,
    /// Towards the last page.
    Next,
}

/// One paging step control: a link when that page exists, an inert disabled
/// button when it does not — the control keeps its place either way (the look
/// the query-builder's result paging established).
///
/// A plain `<a>`, not the router's `<A>`: after hydration the client router
/// intercepts same-origin anchors and pages client-side, and before the WASM
/// bundle loads the browser follows the same href as an ordinary GET, so
/// paging never depends on JavaScript being live.
fn paging_step(href: Option<String>, step: Step) -> AnyView {
    let hook = match step {
        Step::Previous => "prev",
        Step::Next => "next",
    };
    let label = match step {
        Step::Previous => view! {
            <leptos_icons::Icon icon=icondata_lu::LuArrowLeft width="12" height="12" />
            " Previous"
        }
        .into_any(),
        Step::Next => view! {
            "Next "
            <leptos_icons::Icon icon=icondata_lu::LuArrowRight width="12" height="12" />
        }
        .into_any(),
    };
    match href {
        Some(href) => view! {
            <a href=href class=BTN_SECONDARY data-page=hook>
                {label}
            </a>
        }
        .into_any(),
        None => view! {
            <button type="button" class=BTN_SECONDARY disabled=true data-page=hook>
                {label}
            </button>
        }
        .into_any(),
    }
}

/// One page-size choice: a link that re-pages the table at that window size
/// from its first page, or the marked current size.
fn size_choice(base: &str, paging: TablePaging, choice: u32, current: u32) -> AnyView {
    // Two bindings: the view! macro moves child text before evaluating
    // attribute clones, so one String cannot serve both positions.
    let hook = choice.to_string();
    let label = choice.to_string();
    if choice == current {
        return view! {
            <span
                class="rounded-control bg-accent-subtle px-2 py-0.5 font-medium text-accent-ink"
                data-page-size=hook
                aria-current="true"
            >
                {label}
            </span>
        }
        .into_any();
    }
    view! {
        <a
            href=paging.href(base, 0, choice)
            class="rounded-control px-2 py-0.5 text-accent hover:underline"
            data-page-size=hook
        >
            {label}
        </a>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use crate::components::data_table::{
        MAX_PAGE_SIZE, PAGE_SIZE, PageWindow, page_rows, page_window, range_label, row_total,
    };

    fn rows() -> Vec<u32> {
        (0..7).collect()
    }

    #[test]
    fn first_page_starts_at_zero_and_ends_at_the_window() {
        assert_eq!(
            page_window(137, 0, PAGE_SIZE),
            PageWindow {
                page: 0,
                size: 25,
                total: 137,
                start: 0,
                end: 25,
            }
        );
    }

    #[test]
    fn a_middle_page_offsets_by_whole_windows() {
        let window = page_window(137, 2, PAGE_SIZE);
        assert_eq!(window.start, 50);
        assert_eq!(window.end, 75);
        assert_eq!(range_label(window, "templates"), "51–75 of 137 templates");
    }

    #[test]
    fn the_last_page_is_short_and_labelled_by_the_total() {
        let window = page_window(137, 5, PAGE_SIZE);
        assert_eq!(window.start, 125);
        assert_eq!(window.end, 137);
        assert_eq!(range_label(window, "templates"), "126–137 of 137 templates");
    }

    #[test]
    fn a_page_past_the_end_clamps_to_the_last_page_with_rows() {
        // The deleted-last-row case: the reader must land on rows, not on a
        // blank table.
        let window = page_window(26, 9, PAGE_SIZE);
        assert_eq!(window.page, 1);
        assert_eq!(window.start, 25);
        assert_eq!(window.end, 26);
    }

    #[test]
    fn no_rows_is_a_first_class_window() {
        let window = page_window(0, 3, PAGE_SIZE);
        assert_eq!(window.page, 0);
        assert_eq!(window.start, 0);
        assert_eq!(window.end, 0);
        assert_eq!(range_label(window, "stored queries"), "No stored queries");
        assert!(page_rows(&rows(), window).is_empty());
    }

    #[test]
    fn a_hostile_size_is_clamped_not_trusted() {
        // Zero would divide by zero; a huge size would render everything.
        assert_eq!(page_window(100, 0, 0).size, 1);
        assert_eq!(page_window(100, 0, u32::MAX).size, MAX_PAGE_SIZE);
        // …and a clamped size still pages coherently.
        let window = page_window(100, 3, 0);
        assert_eq!(window.start, 3);
        assert_eq!(window.end, 4);
    }

    #[test]
    fn page_rows_slices_the_window_and_never_panics() {
        assert_eq!(page_rows(&rows(), page_window(7, 0, 3)), vec![0, 1, 2]);
        assert_eq!(page_rows(&rows(), page_window(7, 2, 3)), vec![6]);
        // A window computed against a bigger total than the slice holds is
        // empty rather than a panic.
        assert!(page_rows(&rows(), page_window(70, 5, 10)).is_empty());
    }

    #[test]
    fn row_total_saturates_instead_of_wrapping() {
        assert_eq!(row_total(0), 0);
        assert_eq!(row_total(42), 42);
    }
}
