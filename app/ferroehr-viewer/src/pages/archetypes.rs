// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archetypes: the ADL 1.4 and ADL 2 archetypes the CDR keeps beside its
//! templates ([`crate::bff::archetypes`]).
//!
//! The list switches family by `?family=`. An ADL 1.4 archetype opens on its
//! source and its ADL 2 conversion, an ADL 2 archetype on its source, each
//! with its download; Upload takes an ADL 1.4 source. A CDR that keeps no
//! archetypes (`404`) offers none of it. No openEHR spec governs the
//! viewer's pages: our own design.

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{Body, Column, DataTable, Role, data_table, title_cell};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action};
use ferrohealth_ui::view::layout::bullet_list;
use ferrohealth_ui::view::surface::{Crumb, Header, definitions, page_heading};
use ferrohealth_ui::view::tabs::{Tab, TabRow, binder};
use ferrohealth_ui::view::template::list::{ListCard, list_card, list_empty};

use crate::bff::archetypes::{
    Conversion, convert_archetype, fetch_adl14, list_adl2, list_adl14, upload_adl14,
};
use crate::error::ViewerError;
use crate::flash::{Done, redirect_with};
use crate::locale::definitions::DefinitionsMsg;
use crate::locale::{current, tr};
use crate::markup::{h, opt, t};
use crate::pages::common::{
    card_panel, document_section, info_note, is_status, panel, plain_section, refused, section,
};
use crate::routes::Drawn;

/// The route of the list.
pub const ARCHETYPES_ROUTE: &str = "/archetypes";

/// The route of Upload.
const UPLOAD_ROUTE: &str = "/archetypes/upload";

/// The id of Upload's form.
const UPLOAD_FORM: &str = "archetype-upload";

/// An archetype family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// ADL 1.4 archetypes, under `{ext}/definition/archetype/adl1.4`.
    Adl14,
    /// ADL 2 archetypes, under `{ext}/definition/archetype/adl2`.
    Adl2,
}

impl Family {
    /// The family the address names, ADL 1.4 when it names none.
    fn asked() -> Self {
        match crate::request::query("family").as_deref() {
            Some("adl2") => Self::Adl2,
            _ => Self::Adl14,
        }
    }

    /// The family's key in an address.
    #[must_use]
    pub const fn key(self) -> &'static str {
        match self {
            Self::Adl14 => "adl14",
            Self::Adl2 => "adl2",
        }
    }

    /// The family's label.
    fn label(self) -> String {
        tr(match self {
            Self::Adl14 => DefinitionsMsg::ArchetypesAdl14,
            Self::Adl2 => DefinitionsMsg::ArchetypesAdl2,
        })
    }
}

/// The page of the archetype `id` of `family`.
#[must_use]
pub fn archetype_href(family: Family, id: &str) -> String {
    match family {
        Family::Adl14 => format!("{ARCHETYPES_ROUTE}/{}", urlencoding::encode(id)),
        Family::Adl2 => format!("{ARCHETYPES_ROUTE}/adl2/{}", urlencoding::encode(id)),
    }
}

/// The heading of an Archetypes page: `title`, its line, the crumb back to
/// the list of `family` unless it is the list, and its actions.
fn heading(title: String, line: String, crumb: Option<Family>, actions: ActionGroup) -> Html {
    page_heading(
        &current(),
        Header {
            title: title.into(),
            subtitle: Some(line.into()),
            crumbs: crumb
                .map(|family| {
                    vec![Crumb::new(
                        tr(DefinitionsMsg::ArchetypesTitle),
                        format!("{ARCHETYPES_ROUTE}?family={}", family.key()),
                    )]
                })
                .unwrap_or_default(),
            mono: crumb.is_some(),
            badge: None,
            subtitle_view: None,
        },
        actions,
    )
}

/// The note a CDR that keeps no archetypes earns.
fn absent() -> Html {
    card_panel(plain_section(info_note(t(tr(
        DefinitionsMsg::ArchetypesAbsent,
    )))))
}

/// Archetypes, by family.
pub async fn list_page() -> Drawn {
    let family = Family::asked();
    let (adl14, adl2) = futures::join!(list_adl14(), list_adl2());
    let title = tr(DefinitionsMsg::ArchetypesTitle);
    let kept = !matches!((&adl14, &adl2), (Ok(None), Ok(None)));
    let upload = ActionGroup::primary(Action::new(
        Some(Glyph::UPLOAD),
        tr(DefinitionsMsg::ArchetypesUpload),
        Act::Link(UPLOAD_ROUTE.to_owned()),
    ));
    let top = heading(
        title.clone(),
        tr(DefinitionsMsg::ArchetypesSubtitle),
        None,
        if kept { upload } else { ActionGroup::default() },
    );
    if !kept {
        return Drawn::page(title, top.then(absent()));
    }
    let tab = |each: Family, ids: &Result<Option<Vec<String>>, ViewerError>| {
        let tab = Tab::new(
            each.key(),
            format!("{ARCHETYPES_ROUTE}?family={}", each.key()),
            each.label(),
        );
        match ids {
            Ok(Some(ids)) => tab.count(u64::try_from(ids.len()).unwrap_or(u64::MAX), each.label()),
            _ => tab,
        }
    };
    let tabs = vec![tab(Family::Adl14, &adl14), tab(Family::Adl2, &adl2)];
    let shown = match family {
        Family::Adl14 => adl14,
        Family::Adl2 => adl2,
    };
    let body = match shown {
        Ok(Some(ids)) if !ids.is_empty() => id_table(family, &ids),
        Ok(_) => list_empty(tr(DefinitionsMsg::ArchetypesEmpty), None),
        Err(error) => refused(tr(DefinitionsMsg::ArchetypesRefused), &error, None),
    };
    let card = list_card(
        &current(),
        ListCard {
            title: title.clone(),
            count: None,
            order: None,
            tabs: Some(TabRow {
                label: tr(DefinitionsMsg::ArchetypesFamilies),
                tabs,
                current: family.key().to_owned(),
            }),
            actions: None,
            body,
            pager: None,
        },
    );
    Drawn::page(
        title,
        top.then(h(format!(r#"<div class="fui-list-page">{card}</div>"#))),
    )
}

/// The archetype ids of `family`, each row a link to its page.
fn id_table(family: Family, ids: &[String]) -> Html {
    let columns = [Column::text("archetype", tr(DefinitionsMsg::ArchetypeId)).role(Role::Title)];
    let rows: Html = ids
        .iter()
        .map(|id| {
            h(format!(
                "<tr>{}</tr>",
                title_cell(
                    None,
                    Some(&archetype_href(family, id)),
                    h(format!("<code>{}</code>", t(id))),
                    None
                )
            ))
        })
        .collect();
    let caption = family.label();
    data_table(DataTable::new(&caption, &columns, Body::Rows(rows)))
}

/// A conversion, drawn: each root's notes, what the conversion changed, and
/// its ADL 2 source, then the slot fills that joined a template's roots.
#[must_use]
pub fn conversion_panel(conversion: Result<Conversion, ViewerError>) -> Html {
    let conversion = match conversion {
        Ok(conversion) => conversion,
        Err(error) => {
            return panel(plain_section(refused(
                tr(DefinitionsMsg::ConversionRefused),
                &error,
                None,
            )));
        }
    };
    let roots = conversion
        .roots
        .into_iter()
        .enumerate()
        .map(|(place, root)| {
            let notes = (!root.log.notes.is_empty())
                .then(|| bullet_list(root.log.notes.iter().map(t).collect()));
            let details = (!root.conversion_details.is_empty()).then(|| {
                definitions(
                    root.conversion_details
                        .iter()
                        .map(|(key, value)| (key.clone(), t(value)))
                        .collect(),
                )
            });
            section(
                Some(root.archetype_id.clone()),
                None,
                None,
                opt(notes).then(opt(details)),
            )
            .then(document_section(
                tr(DefinitionsMsg::ConversionSource),
                Language::Plain,
                root.adl2,
                &format!("conversion-{place}"),
            ))
        });
    let fills = (!conversion.structure.is_empty()).then(|| {
        let columns = [
            Column::text("parent", tr(DefinitionsMsg::ConversionParent)).role(Role::Title),
            Column::text("path", tr(DefinitionsMsg::ConversionPath)).role(Role::Meta),
            Column::text("child", tr(DefinitionsMsg::ConversionChild)).role(Role::Sub),
        ];
        let rows: Html = conversion
            .structure
            .iter()
            .map(|fill| {
                h(format!(
                    r#"<tr><th scope="row" data-role="title"><code>{}</code></th>{}{}</tr>"#,
                    t(&fill.parent_archetype_id),
                    ferrohealth_ui::table::view::cell(
                        &columns[1],
                        h(format!("<code>{}</code>", t(&fill.parent_path)))
                    ),
                    ferrohealth_ui::table::view::cell(
                        &columns[2],
                        h(format!("<code>{}</code>", t(&fill.child_archetype_id)))
                    ),
                ))
            })
            .collect();
        let caption = tr(DefinitionsMsg::ConversionStructure);
        section(
            Some(caption.clone()),
            None,
            None,
            data_table(DataTable::new(&caption, &columns, Body::Rows(rows))),
        )
    });
    panel(Html::join(roots).then(opt(fills)))
}

/// The tabs of an ADL 1.4 archetype.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchetypeTab {
    /// Its stored source.
    Source,
    /// Its ADL 2 conversion.
    Adl2,
}

/// One ADL 1.4 archetype: its source, or its ADL 2 conversion.
pub async fn page() -> Drawn {
    let id = crate::routes::params()
        .get("archetype_id")
        .unwrap_or_default();
    let tab = match crate::request::query("tab").as_deref() {
        Some("adl2") => ArchetypeTab::Adl2,
        _ => ArchetypeTab::Source,
    };
    let base = archetype_href(Family::Adl14, &id);
    let download = crate::download::archetype_href(Family::Adl14, &id);
    let body = match tab {
        ArchetypeTab::Source => match fetch_adl14(&id).await {
            Ok(Some(source)) => panel(document_section(
                tr(DefinitionsMsg::ArchetypeSource),
                Language::Plain,
                source,
                "archetype-source",
            )),
            Ok(None) => return missing(&id, Family::Adl14),
            Err(error) => panel(plain_section(refused(
                tr(DefinitionsMsg::ArchetypeRefused),
                &error,
                None,
            ))),
        },
        ArchetypeTab::Adl2 => {
            let converted = convert_archetype(&id).await;
            if converted
                .as_ref()
                .is_err_and(|error| is_status(error, http::StatusCode::NOT_FOUND))
            {
                return missing(&id, Family::Adl14);
            }
            conversion_panel(converted)
        }
    };
    let tabs = vec![
        Tab::new("source", base.clone(), tr(DefinitionsMsg::ArchetypeSource)),
        Tab::new(
            "adl2",
            format!("{base}?tab=adl2"),
            tr(DefinitionsMsg::ArchetypeAdl2),
        ),
    ];
    let current_tab = match tab {
        ArchetypeTab::Source => "source",
        ArchetypeTab::Adl2 => "adl2",
    };
    let page = heading(
        id.clone(),
        tr(DefinitionsMsg::ArchetypeAdl14Line),
        Some(Family::Adl14),
        download_action(download),
    )
    .then(binder(
        &current(),
        tr(DefinitionsMsg::ArchetypeViews),
        tabs,
        current_tab,
        body,
    ));
    Drawn::page(id, page)
}

/// One ADL 2 archetype: its source.
pub async fn adl2_page() -> Drawn {
    let id = crate::routes::params()
        .get("archetype_id")
        .unwrap_or_default();
    let body = match crate::bff::template_adl2::fetch_adl2_source(id.clone(), None).await {
        Ok(Some(source)) => panel(document_section(
            tr(DefinitionsMsg::ArchetypeSource),
            Language::Plain,
            source,
            "archetype-source",
        )),
        Ok(None) => return missing(&id, Family::Adl2),
        Err(error) => panel(plain_section(refused(
            tr(DefinitionsMsg::ArchetypeRefused),
            &error,
            None,
        ))),
    };
    let page = heading(
        id.clone(),
        tr(DefinitionsMsg::ArchetypeAdl2Line),
        Some(Family::Adl2),
        download_action(crate::download::archetype_href(Family::Adl2, &id)),
    )
    .then(card_panel(body));
    Drawn::page(id, page)
}

/// The Download action of an archetype at `href`.
fn download_action(href: String) -> ActionGroup {
    ActionGroup::primary(Action::new(
        Some(Glyph::DOWNLOAD),
        tr(DefinitionsMsg::ArchetypeDownload),
        Act::Download(href),
    ))
}

/// The page of an archetype `id` of `family` the CDR does not hold.
fn missing(id: &str, family: Family) -> Drawn {
    crate::request::set_status(http::StatusCode::NOT_FOUND);
    let page = heading(
        id.to_owned(),
        tr(DefinitionsMsg::ArchetypeMissing),
        Some(family),
        ActionGroup::default(),
    );
    Drawn::page(id.to_owned(), page)
}

/// Upload an ADL 1.4 archetype: a success opens it, a refusal stays on the
/// page in the CDR's words, the source as typed.
pub async fn upload_page() -> Drawn {
    let posted = crate::posted::posted();
    let source = posted
        .as_ref()
        .and_then(|posted| posted.get("source-text"))
        .unwrap_or_default();
    let refusal = match posted
        .as_ref()
        .map(crate::posted::Posted::intent)
        .as_deref()
    {
        Some("upload") => match upload_adl14(source.clone()).await {
            Ok(id) => {
                redirect_with(
                    &archetype_href(Family::Adl14, &id),
                    &Done::ok(tr(DefinitionsMsg::ArchetypeUploaded)).line(id),
                );
                return Drawn::default();
            }
            Err(error) => Some(refused(
                tr(DefinitionsMsg::ArchetypeUploadRefused),
                &error,
                None,
            )),
        },
        _ => None,
    };
    let title = tr(DefinitionsMsg::ArchetypeUploadTitle);
    let top = heading(
        title.clone(),
        tr(DefinitionsMsg::ArchetypeUploadSubtitle),
        Some(Family::Adl14),
        ActionGroup::default(),
    );
    let upload = crate::components::source_upload(
        &source,
        tr(DefinitionsMsg::ArchetypeAdl14Source),
        tr(DefinitionsMsg::TemplateChooseFile),
        tr(DefinitionsMsg::ArchetypePlaceholder),
        crate::components::ADL14_SOURCES,
    );
    let form = h(format!(
        r#"<form id="{UPLOAD_FORM}" method="post" action="{UPLOAD_ROUTE}" class="fui-stack">{upload}<div class="fui-cluster"><button type="submit" name="intent" value="upload" class="fui-button fui-button-primary">{glyph}{save}</button></div></form>"#,
        glyph = ferrohealth_ui::icon::icon(Glyph::UPLOAD, "16"),
        save = t(tr(DefinitionsMsg::ArchetypeUploadSave)),
    ));
    Drawn::page(
        title,
        top.then(h(format!(
            r#"<div class="fui-stack">{}{}</div>"#,
            opt(refusal),
            card_panel(plain_section(form))
        ))),
    )
}
