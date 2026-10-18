// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's tables, over FerroUI's table model and pager.
//!
//! One table look (FerroUI's `fui-table` in its scrolling box, a caption naming
//! it for assistive technology), the loading skeleton every listing falls back
//! to, and the pager the paged tables share. Headers are NAMED — raw AQL column
//! indexes (`#0`…) must never reach a header cell.
//!
//! Paging discipline: FerroUI's model (`ferrohealth_ui::table::model`) reads
//! the page (1-based) and size defensively and clamps them, and saturates every
//! offset. A listing that names no patient keeps its position in the URL
//! (`?page=`/`?size=`, [`paging_from_url`]): shareable, walked by back and
//! forward, its pager plain links that work before the WASM bundle loads. A
//! listing that can show patient data keeps it in the page alone
//! ([`paging_in_session`]), never in a URL or the history.

use ferrohealth_ui::table::model::{DEFAULT_SIZE, Paging, Window, href};
use ferrohealth_ui::table::view::link_pager;
use ferrohealth_ui::widget::pager::session_pager;
use leptos::prelude::*;
use leptos_router::params::ParamsMap;

/// Class set for a body row; FerroUI's table draws the separators and the
/// hover tint, so a row needs none of its own.
pub const ROW: &str = "";

/// Class set for a body cell; FerroUI's table pads it.
pub const CELL: &str = "align-top";

/// Class set for a monospace body cell (ids, paths, AQL).
pub const CELL_MONO: &str = "align-top fui-mono";

/// Rows per page across the viewer's tables: FerroUI's default size.
///
/// The AQL fetch window (`fetch`/`_count` on the wire) and the default window
/// of the tables that page rows already in hand, so every listing pages in the
/// same size.
pub const PAGE_SIZE: u32 = DEFAULT_SIZE;

/// The styled table around pre-rendered `<tr>` rows, named `caption`.
///
/// FerroUI's table in its scrolling box, with an explicit `<tbody>`
/// (hydration correctness: browsers insert one otherwise, breaking DOM↔view
/// correspondence). Every header cell carries `scope="col"`, so a screen reader
/// announces the column name with each body cell it reads (the HTML `th`
/// `scope` attribute). The headers are text known only at run time for a
/// result set, so the cells are written here over FerroUI's classes.
#[must_use]
pub fn table_shell(caption: &str, headers: &[&str], body: AnyView) -> AnyView {
    let head = headers
        .iter()
        .map(|h| view! { <th scope="col">{h.to_string()}</th> })
        .collect_view();
    let label = caption.to_owned();
    let caption = caption.to_owned();
    view! {
        <div class="fui-table-scroll" role="region" aria-label=label tabindex="0">
            <table class="fui-table">
                <caption>{caption}</caption>
                <thead>
                    <tr>{head}</tr>
                </thead>
                <tbody>{body}</tbody>
            </table>
        </div>
    }
    .into_any()
}

/// The `<Transition>`/`<Suspense>` fallback every listing shares: FerroUI's
/// skeleton standing in for the rows while the data loads.
#[must_use]
pub fn table_skeleton() -> impl IntoView {
    ferrohealth_ui::view::empty::skeleton(3, "Loading the rows…")
}

/// A paged table's position: where it is kept, and the page and size it names.
#[derive(Debug, Clone, Copy)]
pub struct TablePaging {
    /// The page and size, read from wherever they are kept.
    paging: Signal<Paging>,
    /// Where they are kept: the URL's query, or the page alone.
    source: Source,
}

/// Where a [`TablePaging`] keeps its position.
#[derive(Debug, Clone, Copy)]
enum Source {
    /// The URL's whole query, so a page link keeps every other parameter.
    Url(Memo<ParamsMap>),
    /// The page's own state, which the session pager's buttons set.
    Session(RwSignal<Paging>),
}

impl TablePaging {
    /// The window this position shows over `total` rows: a page past the end
    /// shows the last page, never nothing.
    #[must_use]
    pub fn window(self, total: u32) -> Window {
        Window::over(total, self.paging.get())
    }
}

/// A table's position kept in the URL (`?page=`, `?size=`).
///
/// Call this in SETUP, never inside a `Suspend` closure: the returned position
/// is read where the rows are rendered, so turning the page re-renders the row
/// window without re-running the suspense that fetched the rows.
#[must_use]
pub fn paging_from_url() -> TablePaging {
    let query = leptos_router::hooks::use_query_map();
    let paging = Signal::derive(move || query.with(Paging::from_query));
    TablePaging {
        paging,
        source: Source::Url(query),
    }
}

/// A table's position kept in the page alone, for a listing that can show
/// patient data.
///
/// Call this in SETUP, as [`paging_from_url`].
#[must_use]
pub fn paging_in_session() -> TablePaging {
    let state = RwSignal::new(Paging::default());
    TablePaging {
        paging: state.into(),
        source: Source::Session(state),
    }
}

/// The rows of `window` out of the full row list — the slice the table
/// renders. A window outside the list yields no rows rather than panicking.
#[must_use]
pub fn page_rows<T: Clone>(rows: &[T], window: Window) -> Vec<T> {
    window.slice(rows).to_vec()
}

/// A row count as the fixed-size int the paging math and the pager use.
///
/// WASM is 32-bit and no shared type may carry a `usize`, so a list's
/// `.len()` converts here once, saturating rather than wrapping.
#[must_use]
pub fn row_total(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// FerroUI's pager under a table whose rows are all in hand: which rows are on
/// screen, the steps, and the page-size choices.
///
/// `base` is the screen's own path (`/queries`), `label` names the pager for
/// assistive technology, `paging` comes from [`paging_from_url`] or
/// [`paging_in_session`], and `total` is the (reactive) row count — reactive
/// because a client-side filter changes it. The caller renders its rows through
/// [`TablePaging::window`] + [`page_rows`] from the SAME inputs, so the pager
/// and the rows cannot disagree.
#[must_use]
pub fn table_footer(
    base: &str,
    label: &'static str,
    paging: TablePaging,
    total: Signal<u32>,
) -> AnyView {
    let base = base.to_owned();
    view! {
        <div data-table-footer=label>
            {move || {
                let window = paging.window(total.get());
                match paging.source {
                    Source::Url(query) => {
                        let link = |next: Paging| {
                            let mut query = query.get_untracked();
                            next.write(&mut query);
                            href(&base, &query)
                        };
                        link_pager(label, window, &link).into_any()
                    }
                    Source::Session(state) => session_pager(label, window, state).into_any(),
                }
            }}
        </div>
    }
    .into_any()
}

/// A whole paged table over rows already in hand: the table, the row window,
/// and the pager that pages it.
///
/// `key` is the row's own stable, data-derived identity — never an index.
/// `base` is the screen's own path and `label` names the table and its pager.
#[must_use]
pub fn paged_table<R, V>(
    rows: Vec<R>,
    paging: TablePaging,
    base: &str,
    label: &'static str,
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
        <For each=move || page_rows(&rows, paging.window(total.get())) key=key children=row />
    }
    .into_any();
    let footer = table_footer(base, label, paging, total);
    view! {
        {table_shell(label, headers, body)}
        {footer}
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use ferrohealth_ui::table::model::{Paging, Window};

    use super::{page_rows, row_total};

    fn rows() -> Vec<u32> {
        (0..7).collect()
    }

    #[test]
    fn page_rows_slices_the_window_and_never_panics() {
        let at = |page, size| Window::over(7, Paging { page, size });
        assert_eq!(page_rows(&rows(), at(1, 3)), vec![0, 1, 2]);
        assert_eq!(page_rows(&rows(), at(3, 3)), vec![6]);
        // A page past the end shows the last page, never nothing.
        assert_eq!(page_rows(&rows(), at(9, 3)), vec![6]);
        // A window computed against a bigger total than the slice holds is
        // empty rather than a panic.
        let wide = Window::over(70, Paging { page: 6, size: 10 });
        assert!(page_rows(&rows(), wide).is_empty());
    }

    #[test]
    fn row_total_saturates_instead_of_wrapping() {
        assert_eq!(row_total(0), 0);
        assert_eq!(row_total(42), 42);
    }
}
