// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server › Repository: the whole repository dumped to an archive on the
//! server's file system and loaded from one, behind a confirmation
//! ([`crate::bff::extract`]).
//!
//! The CDR reads and writes the archive at a location on its own file
//! system, so the tab names that location and never moves the archive
//! through the browser. Each answer is the list of entities that did not
//! move. No openEHR spec governs the viewer: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, cell, data_table};
use ferrohealth_ui::view::dialog::{Confirmation, Stakes, confirm_dialog};
use ferrohealth_ui::view::form::Field;

use super::{ServerTab, Tabbed, load_tab, server_page};
use crate::bff::extract::{DumpAsk, Failure, dump, load};
use crate::locale::console::ConsoleMsg;
use crate::locale::operations::OperationsMsg;
use crate::locale::{Msg, current, tr};
use crate::markup::{flag, h, t};
use crate::pages::common::{Refused, info_note, is_status, plain_section, section};
use crate::routes::Drawn;

/// The route of the tab.
const ROUTE: &str = "/server/repository";

/// The id of the dump's form.
const DUMP_FORM: &str = "repository-dump";

/// The id of the load's form.
const LOAD_FORM: &str = "repository-load";

/// The id of Load's confirmation.
const LOAD_DIALOG: &str = "repository-load-confirm";

/// What the post ran.
#[derive(Debug, Clone, PartialEq)]
enum Ran {
    /// Nothing yet.
    Nothing,
    /// A dump, or its refusal.
    Dumped(Result<Vec<Failure>, Refused>),
    /// A load, or its refusal.
    Loaded(Result<Vec<Failure>, Refused>),
}

/// Repository's model.
#[derive(Debug, Clone, PartialEq)]
struct Repository {
    /// The dump's settings as typed.
    dump: DumpAsk,
    /// The load's location as typed.
    location: String,
    /// What ran.
    ran: Ran,
}

/// What the page reads, on the server.
async fn read_repository() -> Tabbed<Repository> {
    load_tab(ServerTab::Repository, || async {
        let posted = crate::posted::posted();
        let text = |key: &str| {
            posted
                .as_ref()
                .map(|posted| posted.text(key))
                .unwrap_or_default()
        };
        let ask = DumpAsk {
            location: text("dump_location"),
            logical_format: text("logical_format"),
            compression: text("compression_format"),
            segment_kb: text("segment_split_size"),
        };
        let location = text("load_location");
        let ran = match posted
            .as_ref()
            .map(crate::posted::Posted::intent)
            .as_deref()
        {
            Some("dump") => Ran::Dumped(
                dump(&ask)
                    .await
                    .map_err(|error| Refused::new(tr(OperationsMsg::DumpRefused), error)),
            ),
            Some("load") => Ran::Loaded(
                load(&location)
                    .await
                    .map_err(|error| Refused::new(tr(OperationsMsg::LoadRefused), error)),
            ),
            _ => Ran::Nothing,
        };
        Repository {
            dump: ask,
            location,
            ran,
        }
    })
    .await
}

/// A text field of the form `form`.
fn text_field(form: &str, id: &str, name: &str, label: OperationsMsg, value: &str) -> Html {
    Field::new(id, tr(label)).holds_id().view(h(format!(
        r#"<input id="{id}" name="{name}" form="{form}" type="text" class="fui-input fui-mono" value="{}" autocomplete="off">"#,
        t(value)
    )))
}

/// A select of the dump's form, its empty choice the CDR's default.
fn choice(id: &str, name: &str, label: OperationsMsg, values: &[&str], chosen: &str) -> Html {
    let options: Html = std::iter::once(("", tr(OperationsMsg::DumpDefault)))
        .chain(values.iter().map(|value| (*value, (*value).to_owned())))
        .map(|(value, words)| {
            h(format!(
                r#"<option value="{value}"{}>{}</option>"#,
                flag("selected", value == chosen),
                t(words)
            ))
        })
        .collect();
    Field::new(id, tr(label)).holds_id().view(h(format!(
        r#"<select id="{id}" name="{name}" form="{DUMP_FORM}" class="fui-select">{options}</select>"#
    )))
}

/// What a dump or a load did not move, or that everything moved.
fn report(failures: &[Failure]) -> Html {
    if failures.is_empty() {
        return info_note(t(tr(OperationsMsg::RepositoryAllMoved)));
    }
    let columns = [
        Column::text("entity", tr(OperationsMsg::RepositoryEntity)).role(Role::Title),
        Column::text("kind", tr(Msg::ColumnKind)).role(Role::Meta),
        Column::text("error", tr(OperationsMsg::RepositoryError)).role(Role::Sub),
    ];
    let rows: Html = failures
        .iter()
        .map(|failure| {
            h(format!(
                r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}</tr>"#,
                t(&failure.entity_id),
                cell(&columns[1], t(&failure.entity_type)),
                cell(&columns[2], t(&failure.error)),
            ))
        })
        .collect();
    let caption = tr(OperationsMsg::RepositoryNotMoved);
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// The answer of a dump or a load in its section: its report or its
/// refusal.
fn answered(answer: Option<&Result<Vec<Failure>, Refused>>) -> Html {
    match answer {
        Some(Ok(failures)) => report(failures),
        Some(Err(refusal)) => refusal.view(),
        None => Html::empty(),
    }
}

/// Repository, drawn.
fn repository(model: Repository) -> Html {
    let Repository {
        dump,
        location,
        ran,
    } = model;
    let refused_405 = |answer: &Result<Vec<Failure>, Refused>| {
        answer
            .as_ref()
            .is_err_and(|refusal| is_status(&refusal.error, http::StatusCode::METHOD_NOT_ALLOWED))
    };
    let (dumped, loaded) = match &ran {
        Ran::Dumped(answer) => (Some(answer), None),
        Ran::Loaded(answer) => (None, Some(answer)),
        Ran::Nothing => (None, None),
    };
    if dumped.or(loaded).is_some_and(refused_405) {
        return plain_section(info_note(t(tr(ConsoleMsg::ServerNoAdmin))));
    }
    let dump_form = h(format!(
        r#"<form id="{DUMP_FORM}" method="post" action="{ROUTE}"></form><div class="fui-transform-options">{location}{format}{compression}{segment}<div class="fui-transform-run"><button type="submit" form="{DUMP_FORM}" name="intent" value="dump" class="fui-button fui-button-primary">{glyph}{save}</button></div></div>"#,
        location = text_field(
            DUMP_FORM,
            "d-location",
            "dump_location",
            OperationsMsg::RepositoryLocation,
            &dump.location
        ),
        format = choice(
            "d-format",
            "logical_format",
            OperationsMsg::DumpFormat,
            &["openehr_canonical_json", "openehr_canonical_xml"],
            &dump.logical_format
        ),
        compression = choice(
            "d-compression",
            "compression_format",
            OperationsMsg::DumpCompression,
            &["zip", "7z"],
            &dump.compression
        ),
        segment = text_field(
            DUMP_FORM,
            "d-segment",
            "segment_split_size",
            OperationsMsg::DumpSegment,
            &dump.segment_kb
        ),
        glyph = ferrohealth_ui::icon::icon(Glyph::DOWNLOAD, "16"),
        save = t(tr(OperationsMsg::Dump)),
    ));
    let load_form = h(format!(
        r#"<form id="{LOAD_FORM}" method="post" action="{ROUTE}"></form><div class="fui-transform-options">{location}<div class="fui-transform-run"><button type="button" class="fui-button fui-button-secondary" command="show-modal" commandfor="{LOAD_DIALOG}" aria-haspopup="dialog">{glyph}{load}</button></div></div>"#,
        location = text_field(
            LOAD_FORM,
            "l-location",
            "load_location",
            OperationsMsg::RepositoryLocation,
            &location
        ),
        glyph = ferrohealth_ui::icon::icon(Glyph::UPLOAD, "16"),
        load = t(tr(OperationsMsg::LoadAsk)),
    ));
    let dialog = confirm_dialog(
        &current(),
        Confirmation {
            id: LOAD_DIALOG.into(),
            title: tr(OperationsMsg::LoadTitle).into(),
            message: tr(OperationsMsg::LoadMessage).into(),
        },
        Stakes::Danger,
        None,
        h(format!(
            r#"<button type="submit" form="{LOAD_FORM}" name="intent" value="load" class="fui-button fui-button-danger">{}{}</button>"#,
            ferrohealth_ui::icon::icon(Glyph::UPLOAD, "16"),
            t(tr(OperationsMsg::Load))
        )),
    );
    Html::join([
        section(
            Some(tr(OperationsMsg::Dump)),
            None,
            None,
            ferrohealth_ui::view::surface::section_intro(tr(OperationsMsg::DumpIntro))
                .then(dump_form)
                .then(answered(dumped)),
        ),
        section(
            Some(tr(OperationsMsg::Load)),
            None,
            None,
            ferrohealth_ui::view::surface::section_intro(tr(OperationsMsg::LoadIntro))
                .then(load_form)
                .then(answered(loaded)),
        ),
        dialog,
    ])
}

/// Server › Repository.
pub async fn page() -> Drawn {
    let tabbed = read_repository().await;
    Drawn::page(
        tr(Msg::NavServer),
        server_page(ServerTab::Repository, tabbed, repository),
    )
}
