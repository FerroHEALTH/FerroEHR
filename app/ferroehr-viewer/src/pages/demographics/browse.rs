// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `/demographics/{kind}` screen — the party browser, one route family for
//! all five kinds.
//!
//! Three cards over one kind: **open a party by id**, **create a party**, and
//! the **demographic tag index**.
//!
//! The lookup is the shape it is because of the wire, not by preference: the
//! released Demographic API publishes no collection `GET`, and AQL's `FROM` is
//! EHR-scoped, so a party is reached by its id
//! ([`super`] module docs, fact 1). The tag index is the one enumerable
//! demographic collection the release does publish
//! (`GET /demographic/tags`) — it spans every kind, because an `ITEM_TAG`
//! reports its target as a bare `UID_BASED_ID` with no kind attached, which is
//! why opening a row asks
//! [`resolve_party_kind`] where that id lives.
//!
//! Every filter, page and window lives in the URL, and the lookup form is a
//! plain `<form method="GET">` so finding a party works before the WASM
//! bundle has loaded.

#![allow(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694); the create skeleton is assembled as that wire's canonical JSON"
)]

use leptos::component;
use leptos::prelude::*;
use leptos_meta::Title;
use leptos_router::components::{A, Redirect};

use crate::components::data_table::{
    CELL, CELL_MONO, ROW, TablePaging, page_rows, page_window, paging_from_url, row_total,
    table_footer, table_shell, table_skeleton,
};
use crate::components::empty_state::EmptyState;
use crate::components::field::{BTN_PRIMARY, BTN_SECONDARY, INPUT, LABEL, TEXTAREA};
use crate::components::item_tags::{ItemTagRow, tag_filter_form};
use crate::components::notice::inline_error;
use crate::components::page_header::PageHeader;
use crate::components::surface::{CARD_PAD, CARD_TITLE};
use crate::components::toast::toast_success;
use crate::error::ViewerError;
use crate::pages::demographics::tags::list_demographic_tags;
use crate::pages::demographics::{PartyKind, browse_href, party_href, resolve_party_kind};
use crate::uid::container_uid_of;

/// `/demographics` — the section's entry point, which opens the default kind's
/// browser.
///
/// A [`Redirect`] rather than a landing page: every surface here is per-kind or
/// per-object, so an index would only hold links the switcher already carries.
/// The default is `PERSON` — the party an operator looks for most, and the
/// switcher reaches the other four in one click.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn DemographicsPage() -> impl IntoView {
    view! {
        <Title text="Demographics" />
        <Redirect path=browse_href(PartyKind::Person) />
    }
}

/// The section's not-found view, for a `:kind` outside the closed five-kind
/// set.
///
/// A route parameter is user input, so an unknown kind is answered with the
/// same "this does not exist" copy the router's own fallback uses, naming the
/// five that do — never a screen full of failing reads against a path segment
/// the CDR has no route for.
#[must_use]
pub fn unknown_kind_view(segment: &str) -> AnyView {
    let shown = segment.to_owned();
    let links = PartyKind::ALL
        .into_iter()
        .map(|kind| {
            view! {
                <A href=browse_href(kind) attr:class="text-accent hover:underline">
                    {kind.plural()}
                </A>
            }
            .into_any()
        })
        .collect::<Vec<_>>();
    view! {
        <Title text="Not found" />
        <div class="p-6">
            <PageHeader title="Unknown party kind" />
            <section class=CARD_PAD id="demographics-unknown-kind">
                <p class="text-sm text-ink">
                    "The demographic API has no party kind called "
                    <span class="font-mono">{shown}</span> ". The five kinds are:"
                </p>
                <div class="mt-2 flex flex-wrap gap-3 text-sm">{links}</div>
            </section>
        </div>
    }
    .into_any()
}

/// The `/demographics/{kind}` screen.
///
/// The `:kind` param is read REACTIVELY, and the whole screen is rebuilt when it
/// changes. That is not optional: the kind switcher and the not-found view's
/// links both navigate to this very route, and `leptos_router` matches a
/// navigation to the same `<Route>` by id — "if two IDs are the same, we do not
/// rerender, but only update the params" (`leptos_router` 0.8
/// `src/nested_router.rs`) — so this body does NOT re-run. Reading the kind once
/// would leave every label, form action, create body and paging base showing the
/// previous kind.
///
/// `?find=<uid>` — what the lookup form submits when WASM has not loaded —
/// short-circuits into a [`Redirect`] to that party's detail route. THAT one is
/// read untracked, because it is a submitted request rather than screen state
/// and can only arrive by full document load. Anything that adds an in-app
/// `?find=` link — or swaps that plain form for the router's
/// `<Form method="GET">`, whose submit is a same-route client-side navigation —
/// must make this decision reactive too.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; view!/mount always consumes the value"
)]
#[component]
pub fn PartyBrowserPage() -> impl IntoView {
    let params = leptos_router::hooks::use_params_map();
    let segment = Memo::new(move |_| params.with(|p| p.get("kind").unwrap_or_default()));
    let kind = Memo::new(move |_| segment.with(|s| PartyKind::from_segment(s)));
    let find = leptos_router::hooks::use_query_map()
        .with_untracked(|q| q.get("find").unwrap_or_default())
        .trim()
        .to_owned();
    // `?find=` arrives only by full document load (see above), so this decision
    // is made once, before the reactive screen exists.
    if !find.is_empty() {
        let target = kind.get_untracked().map(|kind| party_href(kind, &find));
        return match target {
            Some(path) => view! {
                <Title text="Demographics" />
                <Redirect path=path />
            }
            .into_any(),
            None => segment.with_untracked(|segment| unknown_kind_view(segment)),
        };
    }
    view! { {move || kinded_screen(kind, segment, browser_screen)} }.into_any()
}

/// Render one kind's screen, or the not-found view, as a DISTINCT branch per
/// kind.
///
/// The variant is what makes a kind change replace the DOM instead of patching
/// it: two `AnyView`s of the same shape rebuild in place, which leaves an
/// uncontrolled control (the create card's seeded textarea) holding the previous
/// kind's content. `EitherOf6` gives each kind its own branch, which is how
/// divergent branches are expressed.
pub(super) fn kinded_screen<F>(
    kind: Memo<Option<PartyKind>>,
    segment: Memo<String>,
    screen: F,
) -> leptos::either::EitherOf6<AnyView, AnyView, AnyView, AnyView, AnyView, AnyView>
where
    F: Fn(PartyKind) -> AnyView,
{
    use leptos::either::EitherOf6;
    match kind.get() {
        Some(PartyKind::Person) => EitherOf6::A(screen(PartyKind::Person)),
        Some(PartyKind::Organisation) => EitherOf6::B(screen(PartyKind::Organisation)),
        Some(PartyKind::Group) => EitherOf6::C(screen(PartyKind::Group)),
        Some(PartyKind::Agent) => EitherOf6::D(screen(PartyKind::Agent)),
        Some(PartyKind::Role) => EitherOf6::E(screen(PartyKind::Role)),
        None => EitherOf6::F(segment.with(|segment| unknown_kind_view(segment))),
    }
}

/// One kind's browser screen: the switcher, the lookup, the create card and the
/// tag index.
fn browser_screen(kind: PartyKind) -> AnyView {
    let toaster = thaw::ToasterInjection::expect_context();
    let switcher = kind_switcher(kind);
    let lookup = lookup_section(kind);
    let create = create_section(kind, toaster);
    let index = tag_index_section(kind);
    let subtitle = format!(
        "The openEHR demographic API addresses a {} by its id — it publishes no party list, and \
         AQL queries EHRs, not parties. Open one by id, create one, or find one through the tag \
         index.",
        kind.rm_type()
    );

    view! {
        <Title text="Demographics" />
        <div class="p-6">
            <PageHeader title=kind.plural().to_owned() subtitle=subtitle />
            {switcher}
            {lookup}
            {create}
            {index}
        </div>
    }
    .into_any()
}

/// The kind switcher: five pill anchors, one per family. Plain anchors, so
/// switching kind works before hydration (the router intercepts them once WASM
/// loads).
fn kind_switcher(current: PartyKind) -> AnyView {
    let pills = PartyKind::ALL
        .into_iter()
        .map(|kind| {
            let class = if kind == current {
                "rounded-control px-3 py-1.5 text-sm font-medium bg-accent-subtle text-accent-ink"
            } else {
                "rounded-control px-3 py-1.5 text-sm font-medium text-ink-muted hover:bg-sunken"
            };
            view! {
                <a href=browse_href(kind) class=class data-kind=kind.segment()>
                    {kind.plural()}
                </a>
            }
            .into_any()
        })
        .collect::<Vec<_>>();
    view! { <div class="mb-6 flex flex-wrap gap-1 border-b border-edge pb-2">{pills}</div> }
        .into_any()
}

/// The by-id lookup: a plain `<form method="GET">` whose `find` field the page
/// reads and redirects on, so it works with no JavaScript at all; once
/// hydrated, its `on:submit` handler cancels the round trip and navigates
/// client-side instead (identical outcome, one hop instead of two).
///
/// The input is UNCONTROLLED and read at submit: a controlled input resets to
/// its empty signal at hydration, wiping anything typed before the WASM
/// loaded.
fn lookup_section(kind: PartyKind) -> AnyView {
    let lookup_ref = NodeRef::<leptos::html::Input>::new();
    let navigate = leptos_router::hooks::use_navigate();
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let id = lookup_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !id.is_empty() {
            navigate(
                &party_href(kind, &id),
                leptos_router::NavigateOptions::default(),
            );
        }
    };
    let label = format!("{} id", kind.rm_type());
    view! {
        <section class=format!("{CARD_PAD} mb-6")>
            <h2 class=CARD_TITLE>{format!("Open a {}", kind.rm_type())}</h2>
            <form method="GET" action=browse_href(kind) on:submit=on_submit>
                <div class="flex flex-wrap items-end gap-3">
                    <div class="flex flex-col gap-1">
                        <label class=LABEL r#for="party-lookup">
                            {label}
                        </label>
                        <input
                            id="party-lookup"
                            name="find"
                            type="text"
                            class=INPUT
                            placeholder="versioned_object_uid or version_uid"
                            node_ref=lookup_ref
                        />
                    </div>
                    <button id="party-find" type="submit" class=BTN_PRIMARY>
                        "Open"
                    </button>
                </div>
            </form>
            <p class="mt-2 text-xs text-ink-muted">
                "Either id form works: a versioned_object_uid opens the latest version, and a full version_uid is reduced to its container — every screen here addresses the container."
            </p>
        </section>
    }
    .into_any()
}

/// The `ARCHETYPED.rm_version` the skeleton seeds.
///
/// The attribute is "Version of the openEHR reference model used to create this
/// object … e.g. 1.0, 1.2.4" and its only invariant is `Rm_version_valid`:
/// "not `rm_version.is_empty`"
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.archetyped.adoc`).
/// Nothing validates the value further, so this is a starting point the operator
/// replaces with the RM release their own data was built against.
const SEEDED_RM_VERSION: &str = "1.1.0";

/// A minimal RM-valid party document for `kind`, as the create card's starting
/// point.
///
/// Every attribute in it is mandatory, so the skeleton is the smallest party the
/// CDR accepts rather than a guess:
///
/// - `LOCATABLE.name` and `archetype_node_id`
///   (`org.openehr.rm.common.locatable.adoc`);
/// - `archetype_details`, because `PARTY` carries the invariant
///   `Is_archetype_root` and `is_archetype_root` IS `archetype_details /= Void`
///   (`org.openehr.rm.demographic.party.adoc` §Invariants +
///   `…common.locatable.adoc` §Functions) — a party without it is a `422`;
/// - a root `archetype_node_id` equal to the stringified `archetype_id`, since
///   "at an archetype root point, the value of this attribute is always the
///   stringified form of the `archetype_id` found in the `archetype_details`
///   object" (`…common.locatable.adoc` §`archetype_node_id`);
/// - `PARTY.identities` `1..1` with a `PARTY_IDENTITY` carrying its own
///   `name`/`archetype_node_id`/`details`
///   (`org.openehr.rm.demographic.party.adoc`, `…party_identity.adoc`);
/// - for a ROLE additionally `performer` `1..1`, a `PARTY_REF` to "the Version
///   container of Actor playing the role"
///   (`org.openehr.rm.demographic.role.adoc`).
///
/// `name` is the party TYPE, not a person's name: the class documentation says
/// the inherited `name` "is used to indicate the actual type of party (note
/// that the actual names, i.e. identities of parties are indicated in the
/// `identities` attribute)". The archetype ids and the `performer` id are
/// placeholders in the right syntax for the operator to replace — the CDR
/// refuses the ROLE placeholder until it names a real actor, which is the
/// honest behaviour for a template.
#[must_use]
pub fn minimal_party_body(kind: PartyKind) -> String {
    let archetype_id = format!(
        "openEHR-DEMOGRAPHIC-{}.{}.v1",
        kind.rm_type(),
        kind.segment()
    );
    let mut body = serde_json::json!({
        "_type": kind.rm_type(),
        "name": { "_type": "DV_TEXT", "value": kind.rm_type() },
        "archetype_node_id": archetype_id,
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype_id },
            "rm_version": SEEDED_RM_VERSION
        },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "legal identity" },
            "archetype_node_id": "at0001",
            "details": {
                "_type": "ITEM_TREE",
                "name": { "_type": "DV_TEXT", "value": "identity details" },
                "archetype_node_id": "at0002",
                "items": [{
                    "_type": "ELEMENT",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "archetype_node_id": "at0003",
                    "value": { "_type": "DV_TEXT", "value": "" }
                }]
            }
        }]
    });
    if kind == PartyKind::Role
        && let Some(object) = body.as_object_mut()
    {
        drop(object.insert(
            "performer".to_owned(),
            serde_json::json!({
                "_type": "PARTY_REF",
                "namespace": "demographic",
                "type": "PERSON",
                "id": {
                    "_type": "HIER_OBJECT_ID",
                    "value": "replace-with-the-performing-actors-versioned-object-uid"
                }
            }),
        ));
    }
    serde_json::to_string_pretty(&body).unwrap_or_else(|_| body.to_string())
}

/// The create card: the kind's minimal document in a textarea, a Create button
/// dispatching [`create_party`](super::party::create_party), and — on success —
/// a toast plus client-side navigation to the new party's detail route.
///
/// The body is the operator's own canonical JSON, sent verbatim: a create form
/// that assembled a party from viewer-side fields would have to invent the
/// mandatory ones, and a party document is exactly what this screen must not
/// re-model. The textarea is UNCONTROLLED with the skeleton as its child text,
/// so the server and the client render identical markup and typing before
/// hydration is never wiped.
fn create_section(kind: PartyKind, toaster: thaw::ToasterInjection) -> AnyView {
    let body_ref = NodeRef::<leptos::html::Textarea>::new();
    let create: Action<String, Result<String, ViewerError>> = Action::new(move |body: &String| {
        let body = body.clone();
        async move { super::party::create_party(kind.segment().to_owned(), body).await }
    });

    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| match create.value().get() {
        Some(Ok(uid)) => {
            toast_success(
                toaster,
                "Party created",
                &format!("New {} version {uid}", kind.rm_type()),
            );
            navigate(
                &party_href(kind, &uid),
                leptos_router::NavigateOptions::default(),
            );
        }
        Some(Err(error)) => {
            crate::feedback::toast_write_failure(
                toaster,
                "Create failed",
                &format!("the new {}", kind.rm_type()),
                &error,
            );
        }
        None => {}
    });

    let on_click = move |_| {
        let body = body_ref
            .get_untracked()
            .map(|el| el.value())
            .unwrap_or_default();
        if !body.trim().is_empty() {
            create.dispatch(body);
        }
    };
    let skeleton = minimal_party_body(kind);
    view! {
        <section class=format!("{CARD_PAD} mb-6") id="party-create">
            <h2 class=CARD_TITLE>{format!("Create a {}", kind.rm_type())}</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "The document below is the smallest party this kind accepts — every attribute in it is mandatory. Replace the archetype ids and the identity details with your own, then create."
            </p>
            <label class=LABEL r#for="party-create-body">
                {format!("{} document (canonical JSON)", kind.rm_type())}
            </label>
            <textarea
                id="party-create-body"
                class=format!("{TEXTAREA} mt-1 min-h-[18rem]")
                node_ref=body_ref
            >
                {skeleton}
            </textarea>
            <div class="mt-3 flex items-center gap-3">
                <button
                    id="party-create-submit"
                    type="button"
                    class=BTN_PRIMARY
                    disabled=Signal::derive(move || create.pending().get())
                    on:click=on_click
                >
                    <leptos_icons::Icon icon=icondata_lu::LuUserPlus width="14" height="14" />
                    {format!("Create {}", kind.rm_type())}
                </button>
                <Show when=move || create.pending().get()>
                    <span class="text-sm text-ink-muted">"Creating…"</span>
                </Show>
            </div>
            {move || match create.value().get() {
                Some(Err(error)) => {
                    view! { <div class="mt-2">{inline_error(&error)}</div> }.into_any()
                }
                _ => ().into_any(),
            }}
        </section>
    }
    .into_any()
}

/// The demographic tag index: the three released filters as URL state, the
/// matching tags in the shared table kit, and the shared paging footer.
///
/// The filters are a plain `<form method="GET">` submitting to this screen, so
/// filtering works pre-hydration AND its submission is a full document load —
/// which is what keeps [`PartyBrowserPage`]'s untracked `?find=` read sound.
/// The rows are all in hand, so the footer's row math applies
/// ([`page_window`] + [`page_rows`]) and the total is honest.
fn tag_index_section(kind: PartyKind) -> AnyView {
    let query = leptos_router::hooks::use_query_map();
    let filters = Signal::derive(move || {
        query.with(|q| {
            (
                q.get("tag_key").unwrap_or_default(),
                q.get("tag_value").unwrap_or_default(),
                q.get("tag_target_path").unwrap_or_default(),
            )
        })
    });
    let paging = paging_from_url();
    let resource: Resource<Result<Vec<ItemTagRow>, ViewerError>> = Resource::new(
        move || filters.get(),
        |(key, value, path)| async move { list_demographic_tags(key, value, path).await },
    );

    // Opening a row asks the CDR which kind holds that id: an ITEM_TAG names
    // its target but not its type. One action for the whole table, carrying its
    // input so the "nothing holds it" note can name the id.
    let open: Action<String, (String, Result<Option<String>, ViewerError>)> =
        Action::new(|uid: &String| {
            let uid = uid.clone();
            async move {
                let outcome = resolve_party_kind(uid.clone()).await;
                (uid, outcome)
            }
        });
    let navigate = leptos_router::hooks::use_navigate();
    Effect::new(move |_| {
        if let Some((uid, Ok(Some(segment)))) = open.value().get()
            && let Some(kind) = PartyKind::from_segment(&segment)
        {
            navigate(
                &party_href(kind, &uid),
                leptos_router::NavigateOptions::default(),
            );
        }
    });
    let note = move || {
        match open.value().get() {
        Some((uid, Ok(None))) => view! {
            <p class="mt-2 text-sm text-ink-muted">
                {format!(
                    "No party kind holds {uid} — the tag may target an object that was since deleted.",
                )}
            </p>
        }
        .into_any(),
        Some((_, Err(error))) => view! { <div class="mt-2">{inline_error(&error)}</div> }.into_any(),
        _ => ().into_any(),
    }
    };

    let table = view! {
        <Transition fallback=table_skeleton>
            {move || Suspend::new(async move {
                match resource.await {
                    Ok(tags) if tags.is_empty() => {
                        view! {
                            <EmptyState
                                icon=icondata_lu::LuTags
                                message="No demographic tags"
                                hint="Tags are the one demographic collection the openEHR API lists. Set one on a party's Tags tab and it appears here."
                            />
                        }
                            .into_any()
                    }
                    Ok(tags) => tag_index_table(kind, tags, paging, open),
                    Err(e) => inline_error(&e),
                }
            })}
        </Transition>
    }
    .into_any();

    view! {
        <section class=CARD_PAD id="demographic-tag-index">
            <h2 class=CARD_TITLE>"Tag index (all kinds)"</h2>
            <p class="mb-3 text-xs text-ink-muted">
                "The demographic tag list spans every party kind, and a tag names its target without naming that target's kind — so opening a row asks the CDR which kind holds it."
            </p>
            {tag_filter_form(browse_href(kind), filters, &[])}
            {table}
            {note}
        </section>
    }
    .into_any()
}

/// The tag index's table and its shared paging footer, built where the rows are
/// in hand — so the total is a plain value for this render and only the window
/// is reactive (the stored-queries listing's pattern). Turning the page
/// re-windows the rows already fetched; it never refetches.
///
/// `<For>` keyed on the tag identity plus its target, which is unique across
/// the whole demographic space (stable, data-derived, never an index).
fn tag_index_table(
    kind: PartyKind,
    tags: Vec<ItemTagRow>,
    paging: TablePaging,
    open: Action<String, (String, Result<Option<String>, ViewerError>)>,
) -> AnyView {
    let total = row_total(tags.len());
    let rows = view! {
        <For
            each=move || {
                let window = page_window(total, paging.page.get(), paging.size.get());
                page_rows(&tags, window)
            }
            key=ItemTagRow::global_identity
            let:tag
        >
            {tag_index_row(&tag, open)}
        </For>
    }
    .into_any();
    let footer = table_footer(
        &browse_href(kind),
        "tags",
        paging,
        Signal::derive(move || total),
    );
    view! {
        {table_shell(&["Key", "Value", "Target path", "Target", ""], rows)}
        {footer}
    }
    .into_any()
}

/// One tag-index row: its four facts plus the resolve-and-open action.
fn tag_index_row(
    tag: &ItemTagRow,
    open: Action<String, (String, Result<Option<String>, ViewerError>)>,
) -> AnyView {
    let key = tag.key.clone();
    let value = tag.value.clone();
    let target_path = tag.target_path.clone();
    let target = tag.target.clone();
    let shown_target = tag.target.clone();
    let container = container_uid_of(&tag.target);
    view! {
        <tr class=ROW>
            <td class=CELL_MONO>{key}</td>
            <td class=CELL>{value}</td>
            <td class=CELL_MONO>{target_path}</td>
            <td class=CELL_MONO>{shown_target}</td>
            <td class=CELL>
                <button
                    type="button"
                    class=BTN_SECONDARY
                    data-tag-target=target
                    disabled=Signal::derive(move || open.pending().get())
                    on:click=move |_| {
                        open.dispatch(container.clone());
                    }
                >
                    <leptos_icons::Icon icon=icondata_lu::LuEye width="14" height="14" />
                    "Open party"
                </button>
            </td>
        </tr>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::minimal_party_body;
    use crate::pages::demographics::PartyKind;

    #[test]
    fn every_kinds_skeleton_is_a_json_object_of_that_type_with_one_identity() {
        for kind in PartyKind::ALL {
            let body = minimal_party_body(kind);
            let doc: serde_json::Value =
                serde_json::from_str(&body).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
            assert_eq!(doc["_type"], kind.rm_type(), "{kind:?}");
            // PARTY invariant Type_valid: "type = name", and type() is taken
            // from the inherited `name` attribute.
            assert_eq!(doc["name"]["value"], kind.rm_type(), "{kind:?}");
            // PARTY invariant Identities_valid: "not identities.is_empty".
            let identities = doc["identities"]
                .as_array()
                .unwrap_or_else(|| panic!("{kind:?} has no identities array"));
            assert_eq!(identities.len(), 1, "{kind:?}");
            assert_eq!(identities[0]["_type"], "PARTY_IDENTITY", "{kind:?}");
            // PARTY_IDENTITY.details is mandatory on the generated type.
            assert_eq!(identities[0]["details"]["_type"], "ITEM_TREE", "{kind:?}");
            assert!(
                doc["archetype_node_id"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("openEHR-DEMOGRAPHIC-")),
                "{kind:?}"
            );
            // PARTY invariant Is_archetype_root, which IS
            // `archetype_details /= Void` (locatable.adoc §Functions): a party
            // without an ARCHETYPED block is refused, so the skeleton carries
            // one with its mandatory `rm_version`.
            assert_eq!(doc["archetype_details"]["_type"], "ARCHETYPED", "{kind:?}");
            assert!(
                doc["archetype_details"]["rm_version"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty()),
                "{kind:?}"
            );
            // locatable.adoc §archetype_node_id: "at an archetype root point,
            // the value of this attribute is always the stringified form of the
            // archetype_id found in the archetype_details object".
            assert_eq!(
                doc["archetype_node_id"], doc["archetype_details"]["archetype_id"]["value"],
                "{kind:?}"
            );
        }
    }

    #[test]
    fn only_the_role_skeleton_carries_the_mandatory_performer() {
        // ROLE.performer is 1..1 (org.openehr.rm.demographic.role.adoc); no
        // other kind declares it, so seeding one there would be invention.
        let role: serde_json::Value =
            serde_json::from_str(&minimal_party_body(PartyKind::Role)).expect("valid JSON");
        assert_eq!(role["performer"]["_type"], "PARTY_REF");
        assert_eq!(role["performer"]["namespace"], "demographic");
        for kind in PartyKind::ALL.into_iter().filter(|k| *k != PartyKind::Role) {
            let doc: serde_json::Value =
                serde_json::from_str(&minimal_party_body(kind)).expect("valid JSON");
            assert!(doc.get("performer").is_none(), "{kind:?}");
        }
    }
}
