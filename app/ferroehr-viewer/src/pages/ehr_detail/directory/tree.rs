// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The structured directory tree editor.
//!
//! A recursive, editable view of the root `FOLDER` (add / rename / delete
//! folders, add / remove item references) backed by a single working-tree
//! signal, with a dirty-state save bar (PUT with `If-Match`), an advanced
//! raw-JSON mode, and the composition picker for adding `OBJECT_REF` items.
//! Also the shared read-only tree renderer used by the create preview and the
//! history / time / path panels.
//!
//! The working tree is one `RwSignal<serde_json::Value>` seeded from the loaded
//! FOLDER, then stamped with an ephemeral, client-only `_key` identity on every
//! folder and every item reference (`super::edit::stamp_keys`); every mutation
//! goes through the pure [`super::edit`] helpers. `<For>` rows and the collapse
//! / rename / picker UI state are keyed by that stable, data-derived `_key`
//! (never a positional path or index), so a node keeps its own state and row
//! after a sibling delete shifts indices; the live position is re-derived from
//! the `_key` for each read and mutation. The `_key` is stripped
//! (`super::edit::strip_keys`) from every body sent to the CDR and from the
//! advanced-JSON view — it never leaves the viewer.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use serde_json::Value;

use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, TEXTAREA};
use crate::components::format_view::pretty_body;
use crate::components::notice::inline_error;
use crate::components::surface::{CARD_PAD, CARD_TITLE, WELL};
use crate::error::ViewerError;
use crate::format::ReprFormat;
use crate::pages::ehr_detail::directory::edit::{
    add_item, add_subfolder, child_node_keys, count_tree, delete_folder, find_item_index,
    find_path_by_key, item_count, item_node_keys, item_summary, node_key_at, node_name, object_ref,
    remove_item, rename_folder, stamp_keys, strip_keys,
};
use crate::pages::ehr_detail::directory::{DirectoryState, PickerResource};
use crate::pages::ehrs::cell_text;
use crate::uid::container_uid_of;

/// A small square icon button (node actions).
const ICON_BTN: &str = "inline-flex items-center justify-center rounded-control p-1 text-ink-muted hover:bg-sunken hover:text-ink focus:outline-none focus:ring-2 focus:ring-accent";

/// A small square destructive icon button (node delete).
const ICON_BTN_DANGER: &str = "inline-flex items-center justify-center rounded-control p-1 text-ink-muted hover:bg-danger-subtle hover:text-danger focus:outline-none focus:ring-2 focus:ring-danger";

/// Serialize the working tree for the CDR with the ephemeral `_key` identity
/// stripped (contract — viewer-local identity never leaves the BFF).
fn strip_keys_to_string(tree: &Value) -> String {
    let mut stripped = tree.clone();
    strip_keys(&mut stripped);
    serde_json::to_string(&stripped).unwrap_or_default()
}

/// The directory editor's long-lived reactive state, created ONCE in
/// [`directory_section`](super::directory_section) — ABOVE the
/// `<Transition>`/`Suspend` — and re-seeded (idempotent per loaded version) by
/// [`seed`] on each Suspend re-run.
///
/// This is the disposal contract in signal form. A `Suspend` closure re-runs on
/// every notification of the resources it awaits, and each re-run DISPOSES the
/// previous run's reactive owner. Signals created *inside* the Suspend would die
/// while the already-mounted DOM event handlers and icon views still reference
/// them, so the next interaction panics. Held here, above the Suspend at the
/// tab's owner, every signal outlives every re-run.
#[derive(Clone, Copy)]
pub(in crate::pages::ehr_detail::directory) struct EditorState {
    /// The working FOLDER tree (mutated in place; the single source of truth).
    /// Every folder and item reference carries an ephemeral `_key` identity
    /// (see the module doc).
    tree: RwSignal<Value>,
    /// The pristine baseline the working tree is compared against for `dirty`
    /// (the same stamped copy `tree` is seeded from).
    original: RwSignal<Value>,
    /// Ephemeral `_key`s of collapsed folders (expanded unless listed).
    collapsed: RwSignal<std::collections::HashSet<String>>,
    /// The ephemeral `_key` of the folder currently being renamed, if any.
    renaming: RwSignal<Option<String>>,
    /// The in-progress rename text.
    rename_draft: RwSignal<String>,
    /// Whether the advanced raw-JSON editor is open (kept across re-seeds).
    advanced: RwSignal<bool>,
    /// The advanced-mode JSON draft (the stripped tree the user edits).
    json_draft: RwSignal<String>,
    /// The advanced-mode parse error, if the draft is not valid JSON.
    json_error: RwSignal<Option<String>>,
    /// The next free `_key` ordinal, advanced as folders and item references
    /// are added (see [`super::edit::stamp_keys`]); reset to the fresh stamp
    /// count on seed.
    counter: StoredValue<u64>,
    /// The loaded version's `uid.value` (`OBJECT_VERSION_ID`) — the `If-Match`
    /// value the save sends.
    version_uid: RwSignal<String>,
    /// The version this state was last seeded from; [`seed`] is a no-op while
    /// it already equals the loaded version, so a Suspend re-run for the SAME
    /// version never re-parses over the user's in-progress edits.
    seeded_uid: RwSignal<Option<String>>,
    /// The `update` action version captured at seed time; the conflict banner
    /// compares against it so only a failed write AFTER this load counts.
    conflict_baseline: StoredValue<usize>,
    /// The picker modal's manual-entry `OBJECT_REF` namespace. The picker
    /// overlay is an always-mounted hidden `<div>`, so its `prop:value`
    /// closures reference these across Suspend re-runs too — they are hoisted
    /// here for the same disposal reason.
    manual_namespace: RwSignal<String>,
    /// The manual-entry reference `type`.
    manual_type: RwSignal<String>,
    /// The manual-entry `OBJECT_ID` subtype.
    manual_id_type: RwSignal<String>,
    /// The manual-entry id value.
    manual_id: RwSignal<String>,
    /// Whether the working tree differs from `original` (drives the save bar).
    /// Created ONCE at construction — never per Suspend run.
    dirty: Memo<bool>,
    /// Whether a `412` conflict landed on THIS loaded version (drives the
    /// conflict banner). Created ONCE at construction.
    conflicted: Memo<bool>,
}

impl EditorState {
    /// Create the editor's long-lived state. The `dirty` and `conflicted`
    /// memos are created here so they, too, outlive every Suspend re-run;
    /// `update` is the directory-update action they observe.
    pub(in crate::pages::ehr_detail::directory) fn new(
        update: Action<(String, String, String), Result<String, ViewerError>>,
    ) -> Self {
        let tree = RwSignal::new(Value::Null);
        let original = RwSignal::new(Value::Null);
        let counter = StoredValue::new(0u64);
        let conflict_baseline = StoredValue::new(0usize);
        let dirty = Memo::new(move |_| tree.with(|t| original.with(|o| t != o)));
        let conflicted = Memo::new(move |_| {
            update.version().get() > conflict_baseline.get_value()
                && update
                    .value()
                    .with(|v| matches!(v, Some(Err(e)) if super::is_conflict(e)))
        });
        Self {
            tree,
            original,
            collapsed: RwSignal::new(std::collections::HashSet::new()),
            renaming: RwSignal::new(None),
            rename_draft: RwSignal::new(String::new()),
            advanced: RwSignal::new(false),
            json_draft: RwSignal::new(String::new()),
            json_error: RwSignal::new(None),
            counter,
            version_uid: RwSignal::new(String::new()),
            seeded_uid: RwSignal::new(None),
            conflict_baseline,
            manual_namespace: RwSignal::new("local".to_owned()),
            manual_type: RwSignal::new("COMPOSITION".to_owned()),
            manual_id_type: RwSignal::new("HIER_OBJECT_ID".to_owned()),
            manual_id: RwSignal::new(String::new()),
            dirty,
            conflicted,
        }
    }
}

/// Seed [`EditorState`] from the freshly-loaded directory `state`, ONCE per
/// loaded version — the state lives above the Suspend, so a re-run for the same
/// version must NOT re-parse over the user's edits. On a new version it parses
/// the body, stamps a fresh ephemeral `_key` on every folder and item reference
/// ([`super::edit::stamp_keys`]), and resets the working tree, the pristine
/// baseline (the SAME stamped copy, so `dirty` stays a plain equality compare),
/// the counter, `version_uid`, collapse/rename state, the parse error, the
/// advanced-JSON draft and the conflict baseline; `advanced` is left as the user
/// set it.
///
/// # Errors
/// [`ViewerError::Internal`] if the CDR body is not valid JSON (it always is;
/// this is the defensive path — `seeded_uid` is left unset so a later render
/// re-attempts the seed).
pub(in crate::pages::ehr_detail::directory) fn seed(
    editor: &EditorState,
    state: &DirectoryState,
    update: Action<(String, String, String), Result<String, ViewerError>>,
) -> Result<(), ViewerError> {
    if editor.seeded_uid.get_untracked().as_deref() == Some(state.version_uid.as_str()) {
        return Ok(());
    }
    let mut doc: Value = serde_json::from_str(&state.body)
        .map_err(|e| ViewerError::Internal(format!("directory JSON: {e}")))?;
    let mut fresh_counter = 0u64;
    stamp_keys(&mut doc, &mut fresh_counter);
    editor.tree.set(doc.clone());
    editor.original.set(doc);
    editor.counter.set_value(fresh_counter);
    editor.version_uid.set(state.version_uid.clone());
    editor.collapsed.set(std::collections::HashSet::new());
    editor.renaming.set(None);
    editor.json_error.set(None);
    // The advanced-JSON draft shows the STRIPPED tree; the server body already
    // carries no `_key`, so seeding it directly is correct.
    editor
        .json_draft
        .set(pretty_body(&state.body, ReprFormat::CanonicalJson));
    editor
        .conflict_baseline
        .set_value(update.version().get_untracked());
    editor.seeded_uid.set(Some(state.version_uid.clone()));
    Ok(())
}

/// The editor's shared, `Copy` handle threaded through the recursive render:
/// the working tree plus its UI state and the composition picker. Built from
/// the long-lived [`EditorState`] fields — the recursive renderers therefore
/// capture only signals that outlive every Suspend re-run.
#[derive(Clone, Copy)]
struct TreeEditor {
    /// The working FOLDER tree (mutated in place; the single source of truth).
    /// Every folder and item reference carries an ephemeral `_key` identity
    /// (see the module doc).
    tree: RwSignal<Value>,
    /// Ephemeral `_key`s of collapsed folders (expanded unless listed).
    collapsed: RwSignal<std::collections::HashSet<String>>,
    /// The ephemeral `_key` of the folder currently being renamed, if any.
    renaming: RwSignal<Option<String>>,
    /// The in-progress rename text.
    rename_draft: RwSignal<String>,
    /// The next free `_key` ordinal, advanced as folders and item references
    /// are added (see [`super::edit::stamp_keys`]); persisted across clicks for
    /// uniqueness.
    counter: StoredValue<u64>,
    /// The picker modal's manual-entry `OBJECT_REF` namespace (long-lived —
    /// see [`EditorState`]).
    manual_namespace: RwSignal<String>,
    /// The manual-entry reference `type`.
    manual_type: RwSignal<String>,
    /// The manual-entry `OBJECT_ID` subtype.
    manual_id_type: RwSignal<String>,
    /// The manual-entry id value.
    manual_id: RwSignal<String>,
    /// The composition list for the "add item" picker (created outside the
    /// Suspend, read here).
    picker: PickerResource,
    /// The ephemeral `_key` of the folder awaiting an item (also opens the
    /// picker).
    picker_target: RwSignal<Option<String>>,
}

/// The existing-directory experience: the structured tree editor with a
/// dirty-state save bar (`PUT` with `If-Match`), an advanced raw-JSON mode,
/// and the composition picker. ALL reactive state lives in the long-lived
/// [`EditorState`] (seeded by [`seed`] above the Suspend) — this function
/// creates NO signals of its own, so it is safe to re-run on every directory
/// refetch.
#[expect(
    clippy::too_many_lines,
    reason = "the editor view + save bar + advanced mode + picker assembled as one unit"
)]
pub(in crate::pages::ehr_detail::directory) fn tree_editor(
    editor: &EditorState,
    ehr_id: Signal<String>,
    update: Action<(String, String, String), Result<String, ViewerError>>,
    force_save: Action<(String, String), Result<String, ViewerError>>,
    reload: RwSignal<u32>,
    picker: PickerResource,
    picker_target: RwSignal<Option<String>>,
) -> AnyView {
    // `EditorState` is a `Copy` bundle of arena-indexed reactive handles; take
    // an owned copy so the `'static` event-handler closures below capture the
    // long-lived signals directly (the handle is passed by reference only to
    // satisfy `clippy::large_types_passed_by_value`).
    let editor = *editor;
    let dirty = editor.dirty;
    let conflicted = editor.conflicted;

    let ed = TreeEditor {
        tree: editor.tree,
        collapsed: editor.collapsed,
        renaming: editor.renaming,
        rename_draft: editor.rename_draft,
        counter: editor.counter,
        manual_namespace: editor.manual_namespace,
        manual_type: editor.manual_type,
        manual_id_type: editor.manual_id_type,
        manual_id: editor.manual_id,
        picker,
        picker_target,
    };

    // Save the working tree as a new version (PUT + If-Match). The ephemeral
    // `_key` identity is stripped from the wire body.
    let on_save = move |_| {
        let body = editor.tree.with(strip_keys_to_string);
        update.dispatch((ehr_id.get(), editor.version_uid.get_untracked(), body));
    };
    let on_discard = move |_| {
        editor.tree.set(editor.original.get_untracked());
        editor.renaming.set(None);
        editor.json_error.set(None);
    };

    // Advanced mode: toggling on seeds the JSON draft from the current tree
    // (with `_key` stripped — the user never sees viewer-local identity);
    // "Apply" parses it back into the working tree and re-stamps.
    let on_toggle_advanced = move |_| {
        editor.advanced.update(|open| {
            *open = !*open;
            if *open {
                let text = editor.tree.with(|t| {
                    let mut stripped = t.clone();
                    strip_keys(&mut stripped);
                    serde_json::to_string_pretty(&stripped).unwrap_or_else(|_| "{}".to_owned())
                });
                editor.json_draft.set(text);
                editor.json_error.set(None);
            }
        });
    };
    let on_apply_json = move |_| match serde_json::from_str::<Value>(&editor.json_draft.get()) {
        Ok(mut value) => {
            editor.counter.update_value(|c| stamp_keys(&mut value, c));
            editor.tree.set(value);
            editor.json_error.set(None);
        }
        Err(e) => editor.json_error.set(Some(format!("Invalid JSON: {e}"))),
    };

    let tree_body = tree_view(ed);
    let picker_modal = picker_modal(ed);

    view! {
        <section class=CARD_PAD>
            <div class="flex flex-wrap items-center justify-between gap-2">
                <div class="flex items-center gap-2">
                    <h2 class=CARD_TITLE>"Directory"</h2>
                    <span class="text-xs text-ink-muted">
                        {move || {
                            let (folders, items) = editor.tree.with(count_tree);
                            format!("{folders} folders · {items} items")
                        }}
                    </span>
                </div>
                <button
                    id="directory-edit"
                    type="button"
                    class=BTN_SECONDARY
                    on:click=on_toggle_advanced
                >
                    <leptos_icons::Icon icon=icondata_lu::LuCode width="14" height="14" />
                    {move || {
                        if editor.advanced.get() { "Hide JSON" } else { "Advanced: edit as JSON" }
                    }}
                </button>
            </div>

            <div class:hidden=move || editor.advanced.get()>
                <ul id="directory-tree" class="mt-2 text-sm text-ink">
                    {tree_body}
                </ul>
            </div>

            <div class="mt-2 flex flex-col gap-2" class:hidden=move || !editor.advanced.get()>
                <textarea
                    id="directory-body"
                    class=format!("{TEXTAREA} min-h-[16rem]")
                    prop:value=move || editor.json_draft.get()
                    on:input:target=move |ev| editor.json_draft.set(ev.target().value())
                >
                    {editor.json_draft.get_untracked()}
                </textarea>
                <div class="flex items-center gap-3">
                    <button type="button" class=BTN_SECONDARY on:click=on_apply_json>
                        <leptos_icons::Icon icon=icondata_lu::LuCheck width="14" height="14" />
                        "Apply to tree"
                    </button>
                    {move || {
                        editor
                            .json_error
                            .get()
                            .map(|msg| {
                                view! { <span class="text-sm text-danger">{msg}</span> }
                            })
                    }}
                </div>
            </div>

            // The optimistic-concurrency conflict banner: shown after a 412
            // on this loaded version; the unsaved tree is intact.
            <div
                id="directory-conflict"
                class="mt-3 flex flex-wrap items-center gap-3 rounded-control border border-danger/40 bg-danger-subtle px-3 py-2"
                class:hidden=move || !conflicted.get()
            >
                <span class="text-sm text-danger">
                    "This directory changed on the server since it was loaded. Your unsaved edits are still here."
                </span>
                <button
                    type="button"
                    class=BTN_PRIMARY
                    disabled=Signal::derive(move || force_save.pending().get())
                    on:click=move |_| {
                        let body = editor.tree.with(strip_keys_to_string);
                        force_save.dispatch((ehr_id.get(), body));
                    }
                >
                    "Save anyway (overwrite the server change)"
                </button>
                <button
                    type="button"
                    class=BTN_SECONDARY
                    on:click=move |_| reload.update(|n| *n = n.wrapping_add(1))
                >
                    "Discard my edits and load the server version"
                </button>
            </div>

            // Sticky save bar — visible only when the working tree is dirty.
            <div
                class="sticky bottom-0 mt-3 flex flex-wrap items-center gap-3 border-t border-edge bg-raised/95 py-3 backdrop-blur"
                class:hidden=move || !dirty.get()
            >
                <button
                    id="directory-save"
                    type="button"
                    class=BTN_PRIMARY
                    disabled=Signal::derive(move || update.pending().get())
                    on:click=on_save
                >
                    <leptos_icons::Icon icon=icondata_lu::LuSave width="14" height="14" />
                    "Save directory"
                </button>
                <button type="button" class=BTN_SECONDARY on:click=on_discard>
                    "Discard changes"
                </button>
                <Show when=move || update.pending().get()>
                    <span class="text-sm text-ink-muted">"Saving…"</span>
                </Show>
            </div>

            {save_feedback(update)}
            {picker_modal}
        </section>
    }
    .into_any()
}

/// The root of the editable tree (the root folder, addressed by its ephemeral
/// `_key`). The root key is derived REACTIVELY through a `Memo`: the working
/// tree is empty (`Null`) until [`seed`] runs, and this whole editor view is
/// built ONCE — above the directory `Suspend` — so it survives every refetch
/// (; the disposal defect otherwise re-creates the per-folder icon derives on
/// each Suspend re-run). The root's `_key` is stable across seeds
/// (`stamp_keys` always numbers the root `n0`), so the folder tree is built
/// exactly once after the first seed and then updated fine-grained (the inner
/// `<For>`s + reactive reads) — never rebuilt on a plain edit or a re-seed.
fn tree_view(ed: TreeEditor) -> AnyView {
    let root_key = Memo::new(move |_| ed.tree.with(|t| node_key_at(t, &[]).unwrap_or_default()));
    (move || {
        let key = root_key.get();
        if key.is_empty() {
            ().into_any()
        } else {
            render_folder(ed, key, true)
        }
    })
    .into_any()
}

/// Render one FOLDER node, identified by its ephemeral `_key` (`node_key`). Its
/// live positional `path` is re-derived from that `_key` on every reactive read
/// and mutation ([`find_path_by_key`]) so the row stays correct after a sibling
/// delete shifts indices; the collapse / rename UI state and the child folder
/// `<For>` key on `node_key`, and the item `<For>` keys on each item's own
/// ephemeral `_key` ([`item_node_keys`]) — never a positional path or index.
fn render_folder(ed: TreeEditor, node_key: String, is_root: bool) -> AnyView {
    let key_collapsed = node_key.clone();
    let is_collapsed = move || ed.collapsed.with(|c| c.contains(&key_collapsed));

    // Dynamic icons are VIEW branches over static icondata values, never a
    // derived `Signal<Icon>` fed into the `Icon` prop: the icon component's
    // internal reactivity can fire against a row-owned derived signal after the
    // row is disposed and panic-wedge the wasm runtime.
    let key_icon = node_key.clone();
    let folder_icon = move || {
        let expanded = ed.collapsed.with(|c| !c.contains(&key_icon));
        let has_content = ed.tree.with(|t| {
            find_path_by_key(t, &key_icon)
                .is_some_and(|p| !child_node_keys(t, &p).is_empty() || item_count(t, &p) > 0)
        });
        if expanded && has_content {
            view! { <leptos_icons::Icon icon=icondata_lu::LuFolderOpen width="15" height="15" /> }
                .into_any()
        } else {
            view! { <leptos_icons::Icon icon=icondata_lu::LuFolder width="15" height="15" /> }
                .into_any()
        }
    };
    let key_chevron = node_key.clone();
    let chevron = move || {
        if ed.collapsed.with(|c| c.contains(&key_chevron)) {
            view! { <leptos_icons::Icon icon=icondata_lu::LuChevronRight width="14" height="14" /> }
                .into_any()
        } else {
            view! { <leptos_icons::Icon icon=icondata_lu::LuChevronDown width="14" height="14" /> }
                .into_any()
        }
    };

    let key_toggle = node_key.clone();
    let on_toggle = move |_| {
        ed.collapsed.update(|c| {
            if !c.remove(&key_toggle) {
                c.insert(key_toggle.clone());
            }
        });
    };

    let name_area = name_area(ed, node_key.clone());
    let actions = folder_actions(ed, node_key.clone(), is_root);

    let key_children = node_key.clone();
    let key_items = node_key.clone();
    let key_item_rows = node_key;

    view! {
        <li class="py-0.5">
            <div class="flex items-center gap-1">
                <button type="button" class=ICON_BTN aria-label="Toggle folder" on:click=on_toggle>
                    {chevron}
                </button>
                {folder_icon}
                {name_area}
                {actions}
            </div>
            <ul class="ml-2 border-l border-edge pl-4" class:hidden=is_collapsed>
                <For
                    each=move || {
                        ed.tree
                            .with(|t| {
                                find_path_by_key(t, &key_children)
                                    .map(|p| child_node_keys(t, &p))
                                    .unwrap_or_default()
                            })
                    }
                    key=|k: &String| k.clone()
                    let:child_key
                >
                    {render_folder(ed, child_key, false)}
                </For>
                <For
                    each=move || {
                        ed.tree
                            .with(|t| {
                                find_path_by_key(t, &key_items)
                                    .map(|p| item_node_keys(t, &p))
                                    .unwrap_or_default()
                            })
                    }
                    key=|k: &String| k.clone()
                    let:item_key
                >
                    {render_item(ed, key_item_rows.clone(), item_key)}
                </For>
            </ul>
        </li>
    }
    .into_any()
}

/// The name cell: the folder name, or an inline rename input when this folder
/// is being renamed (a single reactive closure over `renaming` + `tree`).
/// Identity is the folder's ephemeral `_key`; the path is re-derived per use.
fn name_area(ed: TreeEditor, node_key: String) -> AnyView {
    let inner = move || {
        if ed
            .renaming
            .with(|r| r.as_deref() == Some(node_key.as_str()))
        {
            let confirm_key = node_key.clone();
            let on_confirm = move |_| {
                let draft = ed.rename_draft.get();
                ed.tree.update(|t| {
                    if let Some(p) = find_path_by_key(t, &confirm_key) {
                        rename_folder(t, &p, draft.trim());
                    }
                });
                ed.renaming.set(None);
            };
            let on_cancel = move |_| ed.renaming.set(None);
            view! {
                <span class="inline-flex items-center gap-1">
                    <input
                        class=INPUT
                        aria-label="Folder name"
                        prop:value=move || ed.rename_draft.get()
                        on:input:target=move |ev| ed.rename_draft.set(ev.target().value())
                    />
                    <button
                        type="button"
                        class=ICON_BTN
                        aria-label="Confirm rename"
                        on:click=on_confirm
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuCheck width="14" height="14" />
                    </button>
                    <button
                        type="button"
                        class=ICON_BTN
                        aria-label="Cancel rename"
                        on:click=on_cancel
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuX width="14" height="14" />
                    </button>
                </span>
            }
            .into_any()
        } else {
            let name_key = node_key.clone();
            let name = ed.tree.with(|t| {
                find_path_by_key(t, &name_key)
                    .map_or_else(|| "(folder)".to_owned(), |p| node_name(t, &p))
            });
            view! { <span class="font-medium text-ink">{name}</span> }.into_any()
        }
    };
    view! { {inner} }.into_any()
}

/// The per-folder action buttons: add subfolder, add item, rename, and (for
/// non-root folders) delete. The folder is identified by its ephemeral `_key`;
/// each mutation re-derives the live path from it before calling the pure
/// path-based [`super::edit`] helpers.
fn folder_actions(ed: TreeEditor, node_key: String, is_root: bool) -> AnyView {
    let add_folder_key = node_key.clone();
    let on_add_folder = move |_| {
        // Append the subfolder, then stamp the (only) unstamped folder with a
        // fresh `_key` so the new row gets a stable identity too.
        ed.counter.update_value(|c| {
            ed.tree.update(|t| {
                if let Some(p) = find_path_by_key(t, &add_folder_key) {
                    add_subfolder(t, &p, "New folder");
                    stamp_keys(t, c);
                }
            });
        });
        ed.collapsed.update(|c| {
            c.remove(&add_folder_key);
        });
    };

    let picker_key = node_key.clone();
    let on_add_item = move |_| ed.picker_target.set(Some(picker_key.clone()));

    let rename_key = node_key.clone();
    let on_rename = move |_| {
        let current = ed.tree.with(|t| {
            find_path_by_key(t, &rename_key).map_or_else(String::new, |p| node_name(t, &p))
        });
        ed.rename_draft.set(current);
        ed.renaming.set(Some(rename_key.clone()));
    };

    let delete_key = node_key;
    let on_delete = move |_| {
        ed.tree.update(|t| {
            if let Some(p) = find_path_by_key(t, &delete_key) {
                delete_folder(t, &p);
            }
        });
    };

    let delete_button = (!is_root).then(|| {
        view! {
            <button
                type="button"
                class=ICON_BTN_DANGER
                aria-label="Delete folder"
                title="Delete folder"
                on:click=on_delete
            >
                <leptos_icons::Icon icon=icondata_lu::LuTrash width="14" height="14" />
            </button>
        }
    });

    view! {
        <span class="ml-1 flex items-center gap-0.5">
            <button
                type="button"
                class=ICON_BTN
                aria-label="Add subfolder"
                title="Add subfolder"
                on:click=on_add_folder
            >
                <leptos_icons::Icon icon=icondata_lu::LuFolderPlus width="14" height="14" />
            </button>
            <button
                type="button"
                class=ICON_BTN
                aria-label="Add item reference"
                title="Add item reference"
                on:click=on_add_item
            >
                <leptos_icons::Icon icon=icondata_lu::LuFilePlus width="14" height="14" />
            </button>
            <button
                type="button"
                class=ICON_BTN
                aria-label="Rename folder"
                title="Rename folder"
                on:click=on_rename
            >
                <leptos_icons::Icon icon=icondata_lu::LuPencil width="14" height="14" />
            </button>
            {delete_button}
        </span>
    }
    .into_any()
}

/// One item reference row: the ref type + id value, with a remove button.
/// Identified by the item's OWN ephemeral `_key` (`item_key`) inside the owning
/// folder's `_key` (`parent_key`) — never the item's position, which every
/// removal of an earlier sibling shifts. Both live positions are re-derived per
/// use: the folder path from `parent_key` ([`find_path_by_key`]), the item index
/// from `item_key` ([`find_item_index`]).
///
/// The row carries its referenced id as `data-item-id`, so a reader can name
/// WHICH reference a row is rather than counting rows.
fn render_item(ed: TreeEditor, parent_key: String, item_key: String) -> AnyView {
    let summary_folder = parent_key.clone();
    let summary_item = item_key.clone();
    let (ref_type, id) = ed.tree.with(|t| {
        find_path_by_key(t, &summary_folder)
            .and_then(|p| find_item_index(t, &p, &summary_item).map(|i| item_summary(t, &p, i)))
            .unwrap_or_else(|| ("OBJECT".to_owned(), "(ref)".to_owned()))
    });
    let on_remove = move |_| {
        ed.tree.update(|t| {
            if let Some(p) = find_path_by_key(t, &parent_key)
                && let Some(idx) = find_item_index(t, &p, &item_key)
            {
                remove_item(t, &p, idx);
            }
        });
    };
    let hook = id.clone();
    view! {
        <li class="flex items-center gap-1 py-0.5 text-ink-muted" data-item-id=hook>
            <leptos_icons::Icon icon=icondata_lu::LuFileText width="14" height="14" />
            <span class="mr-1 text-xs uppercase">{ref_type}</span>
            <span class="break-all font-mono text-xs">{id}</span>
            <button
                type="button"
                class=ICON_BTN_DANGER
                aria-label="Remove item"
                title="Remove item"
                on:click=on_remove
            >
                <leptos_icons::Icon icon=icondata_lu::LuX width="12" height="12" />
            </button>
        </li>
    }
    .into_any()
}

/// The "add item reference" picker: an overlay revealed when a folder's add
/// button sets `picker_target`. Offers the EHR's compositions (first page,
/// from the shared picker resource) and a manual `OBJECT_REF` entry form.
#[expect(
    clippy::too_many_lines,
    reason = "the modal composes the composition list + the manual-entry form as one overlay"
)]
fn picker_modal(ed: TreeEditor) -> AnyView {
    // The manual-entry form signals are long-lived (on [`EditorState`]) — this
    // overlay is an always-mounted hidden `<div>`, so its `prop:value` closures
    // must reference signals that outlive every Suspend re-run.
    let manual_namespace = ed.manual_namespace;
    let manual_type = ed.manual_type;
    let manual_id_type = ed.manual_id_type;
    let manual_id = ed.manual_id;

    let close = move |_| ed.picker_target.set(None);

    let add_manual = move |_| {
        if let Some(key) = ed.picker_target.get() {
            let item = object_ref(
                manual_namespace.get().trim(),
                manual_type.get().trim(),
                manual_id_type.get().trim(),
                manual_id.get().trim(),
            );
            // Append the reference, then stamp the (only) unstamped node with a
            // fresh `_key` so the new item row gets a stable identity too.
            ed.counter.update_value(|c| {
                ed.tree.update(|t| {
                    if let Some(p) = find_path_by_key(t, &key) {
                        add_item(t, &p, item);
                        stamp_keys(t, c);
                    }
                });
            });
            ed.collapsed.update(|c| {
                c.remove(&key);
            });
            manual_id.set(String::new());
            ed.picker_target.set(None);
        }
    };

    let composition_list = view! {
        <Transition fallback=|| {
            view! { <p class="text-sm text-ink-muted">"Loading compositions…"</p> }
        }>
            {move || Suspend::new(async move {
                match ed.picker.await {
                    Ok(Some(page)) => composition_choices(ed, &page.rows),
                    Ok(None) => ().into_any(),
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();

    view! {
        <div
            class="fixed inset-0 z-40 flex items-start justify-center bg-scrim p-4 sm:items-center"
            class:hidden=move || ed.picker_target.with(Option::is_none)
        >
            <div class="max-h-[85vh] w-full max-w-xl overflow-auto rounded-card border border-edge bg-raised p-4 shadow-card">
                <div class="mb-3 flex items-center justify-between">
                    <h3 class=CARD_TITLE>"Add item reference"</h3>
                    <button type="button" class=ICON_BTN aria-label="Close" on:click=close>
                        <leptos_icons::Icon icon=icondata_lu::LuX width="16" height="16" />
                    </button>
                </div>

                <p class="mb-2 text-sm font-medium text-ink">"This EHR's compositions"</p>
                <div class="mb-4 flex flex-col gap-1">{composition_list}</div>

                <p class="mb-2 text-sm font-medium text-ink">"Or enter a reference manually"</p>
                <div class="grid grid-cols-1 gap-2 sm:grid-cols-2">
                    <input
                        class=INPUT
                        aria-label="Namespace"
                        placeholder="namespace (e.g. local)"
                        prop:value=move || manual_namespace.get()
                        on:input:target=move |ev| manual_namespace.set(ev.target().value())
                    />
                    <input
                        class=INPUT
                        aria-label="Reference type"
                        placeholder="type (e.g. COMPOSITION)"
                        prop:value=move || manual_type.get()
                        on:input:target=move |ev| manual_type.set(ev.target().value())
                    />
                    <input
                        class=INPUT
                        aria-label="Id type"
                        placeholder="id type (e.g. HIER_OBJECT_ID)"
                        prop:value=move || manual_id_type.get()
                        on:input:target=move |ev| manual_id_type.set(ev.target().value())
                    />
                    <input
                        class=INPUT
                        aria-label="Id value"
                        placeholder="id value"
                        prop:value=move || manual_id.get()
                        on:input:target=move |ev| manual_id.set(ev.target().value())
                    />
                </div>
                <div class="mt-3 flex justify-end gap-2">
                    <button type="button" class=BTN_SECONDARY on:click=close>
                        "Cancel"
                    </button>
                    <button
                        type="button"
                        class=BTN_PRIMARY
                        disabled=Signal::derive(move || manual_id.with(|v| v.trim().is_empty()))
                        on:click=add_manual
                    >
                        <leptos_icons::Icon icon=icondata_lu::LuPlus width="14" height="14" />
                        "Add reference"
                    </button>
                </div>
            </div>
        </div>
    }
    .into_any()
}

/// The composition rows in the picker: each adds a `COMPOSITION` `OBJECT_REF`
/// (namespace `local`, `HIER_OBJECT_ID` = the versioned-object id) to the
/// target folder.
fn composition_choices(ed: TreeEditor, rows: &[Vec<Value>]) -> AnyView {
    if rows.is_empty() {
        // An inline hint, not an EmptyState: this is one half of a compact
        // picker overlay whose other half ("or enter a reference manually")
        // remains fully usable, so the region is not a void — and a dashed box
        // inside a modal reads as a second, broken dialog.
        return view! { <p class="text-sm text-ink-muted">"No compositions in this EHR yet."</p> }
            .into_any();
    }
    let rows = rows.to_vec();
    view! {
        <For
            each=move || rows.clone()
            key=|row| row.first().map(cell_text).unwrap_or_default()
            let:row
        >
            {composition_choice(ed, &row)}
        </For>
    }
    .into_any()
}

/// One composition choice row.
fn composition_choice(ed: TreeEditor, row: &[Value]) -> AnyView {
    let uid = row.first().map(cell_text).unwrap_or_default();
    let name = row.get(1).map(cell_text).unwrap_or_default();
    let object_id = container_uid_of(&uid);
    let uid_display = uid.clone();
    let on_add = move |_| {
        if let Some(key) = ed.picker_target.get() {
            let item = object_ref("local", "COMPOSITION", "HIER_OBJECT_ID", &object_id);
            // Stamp the appended reference so its row has a stable identity
            // (the same discipline as the manual-entry path above).
            ed.counter.update_value(|c| {
                ed.tree.update(|t| {
                    if let Some(p) = find_path_by_key(t, &key) {
                        add_item(t, &p, item);
                        stamp_keys(t, c);
                    }
                });
            });
            ed.collapsed.update(|c| {
                c.remove(&key);
            });
            ed.picker_target.set(None);
        }
    };
    view! {
        <button
            type="button"
            class="flex w-full items-center justify-between gap-2 rounded-control border border-edge px-3 py-1.5 text-left text-sm hover:bg-sunken"
            on:click=on_add
        >
            <span class="min-w-0">
                <span class="block truncate text-ink">{name}</span>
                <span class="block truncate font-mono text-xs text-ink-muted">{uid_display}</span>
            </span>
            <leptos_icons::Icon icon=icondata_lu::LuPlus width="14" height="14" />
        </button>
    }
    .into_any()
}

/// The save action's failure pane: the CDR's diagnostics verbatim in a
/// scrollable WELL (`<pre>`). Success is a toast (see the section orchestrator).
fn save_feedback(update: Action<(String, String, String), Result<String, ViewerError>>) -> AnyView {
    view! {
        {move || match update.value().get() {
            Some(Err(error)) => {
                view! {
                    <div class=format!("{WELL} mt-3")>
                        <pre class="max-h-[40vh] overflow-auto whitespace-pre-wrap font-mono text-xs text-danger">
                            {error.to_string()}
                        </pre>
                    </div>
                }
                    .into_any()
            }
            _ => ().into_any(),
        }}
    }
    .into_any()
}

/// A read-only FOLDER tree (the create preview and the history / time / path
/// panels): folders and their item references, no actions.
pub(in crate::pages::ehr_detail::directory) fn read_only_tree(folder: &Value) -> AnyView {
    view! { <ul class="text-sm text-ink">{read_only_folder(folder)}</ul> }.into_any()
}

/// One read-only FOLDER node.
fn read_only_folder(folder: &Value) -> AnyView {
    let name = folder
        .get("name")
        .and_then(|n| n.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("(folder)")
        .to_owned();
    let subfolders = folder
        .get("folders")
        .and_then(Value::as_array)
        .map(|folders| folders.iter().map(read_only_folder).collect::<Vec<_>>())
        .unwrap_or_default();
    let items = folder
        .get("items")
        .and_then(Value::as_array)
        .map(|items| items.iter().map(read_only_item).collect::<Vec<_>>())
        .unwrap_or_default();
    view! {
        <li class="py-0.5">
            <span class="inline-flex items-center gap-1.5 font-medium text-ink">
                <leptos_icons::Icon icon=icondata_lu::LuFolder width="14" height="14" />
                {name}
            </span>
            <ul class="ml-2 border-l border-edge pl-4">{subfolders} {items}</ul>
        </li>
    }
    .into_any()
}

/// One read-only `OBJECT_REF` item.
fn read_only_item(item: &Value) -> AnyView {
    let ref_type = item
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("OBJECT")
        .to_owned();
    let id = item
        .get("id")
        .and_then(|i| i.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("(ref)")
        .to_owned();
    view! {
        <li class="flex items-center gap-1.5 py-0.5 text-ink-muted">
            <leptos_icons::Icon icon=icondata_lu::LuFileText width="14" height="14" />
            <span class="text-xs uppercase">{ref_type}</span>
            <span class="break-all font-mono text-xs">{id}</span>
        </li>
    }
    .into_any()
}
