// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Templates: the list with its two families, Upload a template, one
//! template with its tabs, and its delete (FerroUI
//! `design/consoles/ferroehr/`, pages 16.1 to 16.16).
//!
//! An ADL 1.4 template is read once as its operational template; the Paths
//! tab browses the path catalog distilled from its Web Template (E5), and
//! the Example tab asks the CDR for an example composition (ITS-REST
//! `template_example_get`). An ADL 2 template carries its release in its
//! HRID; the page reads the release the address names, the latest for none,
//! and builds its paths from the operational JSON, since the CDR has no Web
//! Template for ADL 2. The family is the address's `family`, the tab its
//! `tab`. Each page reads the CDR and answers its post on the server, into
//! one model that both halves draw. No openEHR spec governs the viewer's
//! pages: our own design.

use std::collections::BTreeSet;

use ferrohealth_ui::highlight::Language;
use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::filter::{Find, find_form};
use ferrohealth_ui::table::model::StateSource;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, menu_cell, title_cell,
};
use ferrohealth_ui::view::action::ActionGroup;
use ferrohealth_ui::view::action_menu::{Act, Action, Item, Menu, row_menu};
use ferrohealth_ui::view::browser::{BrowserPane, Heading, Panes, PathNode, browser, path_node};
use ferrohealth_ui::view::coding::{Coding, Layout, coding};
use ferrohealth_ui::view::diagnostics::diagnostics;
use ferrohealth_ui::view::dialog::{ConfirmForm, Confirmation, danger_form_dialog};
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::segmented::segmented_links;
use ferrohealth_ui::view::surface::{
    Crumb, Header, Mode, definitions, editor_heading, page_heading,
};
use ferrohealth_ui::view::tabs::{Tab, TabRow, binder};
use ferrohealth_ui::view::template::list::{
    ActiveFilters, FilterChip, ListCard, active_filters, list_card, list_empty,
};
use ferrohealth_ui::view::tree::{Tree, TreeKind, TreeNode, TreeWords, tree_view};

use crate::adl2::family_versions;
use crate::adl2::{TemplateFamily, split_hrid};
use crate::bff::template_adl2::{
    fetch_adl2_catalog, fetch_adl2_example, fetch_adl2_json, fetch_adl2_source,
};
use crate::bff::template_detail::TemplateDetail;
use crate::bff::template_detail::{fetch_example, fetch_template_detail};
use crate::bff::templates::TemplateRow;
use crate::bff::templates::{
    delete_adl2_artefact, list_adl2_templates, list_templates, upload_adl2_template,
    upload_template,
};
use crate::builder::catalog::CatalogNode;
use crate::components::source_upload;
use crate::error::ViewerError;
use crate::example_options::{ExampleDetail, ExampleType};
use crate::flash::{Done, redirect_with};
use crate::format::ReprFormat;
use crate::locale::definitions::DefinitionsMsg;
use crate::locale::{Msg, current, message, tr, tr_with};
use crate::markup::{flag, h, opt, t};
use crate::pages::common::discarded;
use crate::pages::common::{
    EditorDock, ago, card_panel, count, dock_editor, document_section, findings_of, info_note,
    panel, plain_section, refused, section, when,
};
use crate::routes::Drawn;

/// The id of Delete template's confirmation dialog.
const DELETE_DIALOG: &str = "delete-template";

/// The page of the ADL 1.4 template `template_id`.
#[must_use]
pub fn template_href(template_id: &str) -> String {
    format!("/templates/{}", urlencoding::encode(template_id))
}

/// The page of the ADL 2 template `hrid`, at `release` when one is named.
#[must_use]
pub fn adl2_href(stem: &str, release: Option<&str>) -> String {
    match release {
        Some(release) => format!(
            "/templates/adl2/{}/{}",
            urlencoding::encode(stem),
            urlencoding::encode(release)
        ),
        None => format!("/templates/adl2/{}", urlencoding::encode(stem)),
    }
}

/// The query builder over `template_id`, at `path` when one is named.
fn query_href(template_id: &str, path: Option<&str>) -> String {
    let mut href = format!("/query?template={}", urlencoding::encode(template_id));
    if let Some(path) = path {
        href.push_str("&path=");
        href.push_str(&urlencoding::encode(path));
    }
    href
}

/// The family the address names.
fn asked_family() -> TemplateFamily {
    crate::request::query("family")
        .as_deref()
        .map_or(TemplateFamily::Adl14, TemplateFamily::from_query)
}

/// The templates of one family as a table, each with its row menu and its
/// delete confirmation.
fn rows_table(family: TemplateFamily, rows: &[TemplateRow]) -> Html {
    let columns = [
        Column::text("template", tr(DefinitionsMsg::ColumnTemplate)).role(Role::Title),
        Column::text("root", tr(DefinitionsMsg::ColumnRootArchetype)).role(Role::Sub),
        Column::text("uploaded", tr(DefinitionsMsg::ColumnUploaded)).role(Role::Meta),
        Column::text("menu", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let body = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let (href, name) = match family {
                TemplateFamily::Adl14 => (template_href(&row.template_id), t(&row.template_id)),
                TemplateFamily::Adl2 => {
                    let (stem, release) = split_hrid(&row.template_id);
                    (
                        adl2_href(stem, release),
                        h(format!("<code>{}</code>", t(&row.template_id))),
                    )
                }
            };
            let sub = h(format!(
                r#"<span class="fui-row-sub">{}</span>"#,
                t(&row.concept)
            ));
            let confirm = format!(
                "{href}{}confirm=delete",
                if href.contains('?') { '&' } else { '?' }
            );
            let menu = row_menu(
                &current(),
                &Menu::new(
                    format!("template-{index}"),
                    tr_with(
                        Msg::TemplateActionsFor,
                        &[("name", row.template_id.clone().into())],
                    ),
                    row.is_composition()
                        .then(|| {
                            Item::new(Action::new(
                                Some(Glyph::SQUARE_TERMINAL),
                                tr(DefinitionsMsg::TemplateBuildQuery),
                                Act::Link(query_href(&row.template_id, None)),
                            ))
                        })
                        .into_iter()
                        .collect(),
                )
                .destructive(vec![Item::new(Action::new(
                    Some(Glyph::TRASH),
                    tr(DefinitionsMsg::TemplateDeleteRow),
                    Act::Link(confirm),
                ))]),
            );
            h(format!(
                "<tr>{}{}{}{}</tr>",
                title_cell(Some(&href), name, Some(sub)),
                cell(
                    &columns[1],
                    h(format!("<code>{}</code>", t(&row.archetype_id)))
                ),
                cell(&columns[2], ago(&row.created)),
                menu_cell(menu)
            ))
        })
        .collect();
    let caption = tr(family.label());
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Rows(body),
    })
}

/// The confirmation of deleting the template `template_id` of `family`,
/// posting to `action`.
fn delete_dialog(
    family: TemplateFamily,
    template_id: &str,
    id: &'static str,
    action: &str,
) -> Html {
    danger_form_dialog(
        &current(),
        Confirmation {
            id: id.into(),
            title: tr_with(
                DefinitionsMsg::TemplateDeleteTitle,
                &[("name", template_id.to_owned().into())],
            )
            .into(),
            message: tr(DefinitionsMsg::TemplateDeleteMessage).into(),
        },
        ConfirmForm {
            action: action.to_owned(),
            fields: delete_fields(family, template_id),
        },
        delete_submit(),
    )
}

/// The fields a template's delete posts: its intent, its family and its id.
fn delete_fields(family: TemplateFamily, template_id: &str) -> Html {
    h(format!(
        r#"<input type="hidden" name="intent" value="delete-template"><input type="hidden" name="family" value="{}"><input type="hidden" name="template" value="{}">"#,
        family.as_query(),
        t(template_id)
    ))
}

/// The submit of a template's delete.
fn delete_submit() -> Html {
    h(format!(
        r#"<button type="submit" class="fui-button fui-button-danger">{}{}</button>"#,
        ferrohealth_ui::icon::icon(Glyph::TRASH, "16"),
        t(tr(DefinitionsMsg::TemplateDeleteConfirm))
    ))
}

/// The delete confirmation drawn in the page, for a reader who came from a
/// list row's Delete… (the address's `confirm=delete`).
// NOTE: FerroUI `view::dialog::Confirmation::id` is `&'static str`, so a list draws no dialog per row.
fn inline_confirm(family: TemplateFamily, template_id: &str, action: &str) -> Option<Html> {
    (crate::request::query("confirm").as_deref() == Some("delete")).then(|| {
        crate::pages::common::warn_note(h(format!(
            r#"<p><strong>{title}</strong> {message}</p><form method="post" action="{action}" class="fui-cluster">{fields}{submit}<a class="fui-button fui-button-secondary" href="{action}">{cancel}</a></form>"#,
            title = t(tr_with(
                DefinitionsMsg::TemplateDeleteTitle,
                &[("name", template_id.to_owned().into())],
            )),
            message = t(tr(DefinitionsMsg::TemplateDeleteMessage)),
            action = t(action),
            fields = delete_fields(family, template_id),
            submit = delete_submit(),
            cancel = t(tr(Msg::Cancel)),
        )))
    })
}

/// Deletes the template a post names and sends the browser to `back`; `Some`
/// holds the CDR's refusal.
async fn delete_posted(back: &str) -> Option<ViewerError> {
    let posted = crate::posted::posted_for("delete-template")?;
    let template = posted.text("template");
    let done = match TemplateFamily::from_query(&posted.text("family")) {
        TemplateFamily::Adl14 => crate::admin::admin_delete_template(template.clone()).await,
        TemplateFamily::Adl2 => delete_adl2_artefact(template.clone()).await,
    };
    match done {
        Ok(()) => {
            redirect_with(
                back,
                &Done::ok(tr(DefinitionsMsg::TemplateDeleted)).line(template),
            );
            None
        }
        Err(error) => Some(error),
    }
}

/// The refusal of a delete the page answered.
fn delete_refused(acted: Option<&ViewerError>) -> Option<Html> {
    acted.map(|error| refused(tr(DefinitionsMsg::TemplateDeleteRefused), error, None))
}

/// What Templates read: the delete it answered and both families' lists.
#[derive(Debug, Clone)]
pub struct Templates {
    /// The CDR's refusal of the delete posted.
    acted: Option<ViewerError>,
    /// The ADL 1.4 templates.
    adl14: Result<Vec<TemplateRow>, ViewerError>,
    /// The ADL 2 templates.
    adl2: Result<Vec<TemplateRow>, ViewerError>,
}

/// Answers a delete posted to Templates, which returns to `family`, and reads
/// both families' lists.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
pub async fn load_templates(
    // The family the page shows.
    family: TemplateFamily,
) -> Result<Templates, ViewerError> {
    crate::session::require_session()?;
    let acted = delete_posted(&format!("/templates?family={}", family.as_query())).await;
    let (adl14, adl2) = (list_templates().await, list_adl2_templates().await);
    Ok(Templates { acted, adl14, adl2 })
}

/// Templates.
/// Templates, by family.
#[expect(
    clippy::too_many_lines,
    reason = "one page: the model it reads and the view it draws"
)]
pub async fn list_page() -> Drawn {
    let family = asked_family();
    let find = crate::request::query("q").unwrap_or_default();
    let heading = page_heading(
        &current(),
        Header {
            title: tr(Msg::NavTemplates).into(),
            subtitle: Some(tr(DefinitionsMsg::TemplatesSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::primary(Action::new(
            Some(Glyph::UPLOAD),
            tr(DefinitionsMsg::TemplatesUpload),
            Act::Link(format!("/templates/upload?family={}", family.as_query())),
        )),
    );
    let Templates { acted, adl14, adl2 } = match load_templates(family).await {
        Ok(templates) => templates,
        Err(error) => {
            return Drawn::page(
                tr(Msg::NavTemplates),
                heading.then(refused(tr(DefinitionsMsg::TemplatesRefused), &error, None)),
            );
        }
    };
    let acted = delete_refused(acted.as_ref());
    let needle = find.to_lowercase();
    let keep =
        |rows: Result<Vec<TemplateRow>, ViewerError>| -> Result<Vec<TemplateRow>, ViewerError> {
            rows.map(|rows| {
                rows.into_iter()
                    .filter(|row| {
                        needle.is_empty()
                            || row.template_id.to_lowercase().contains(&needle)
                            || row.concept.to_lowercase().contains(&needle)
                            || row.archetype_id.to_lowercase().contains(&needle)
                    })
                    .collect()
            })
        };
    let (adl14, adl2) = (keep(adl14), keep(adl2));
    let size = |rows: &Result<Vec<TemplateRow>, ViewerError>| {
        rows.as_ref()
            .ok()
            .map(|rows| u64::try_from(rows.len()).unwrap_or(u64::MAX))
    };
    let tab = |each: TemplateFamily, rows: &Result<Vec<TemplateRow>, ViewerError>| {
        let mut href = format!("/templates?family={}", each.as_query());
        if !find.is_empty() {
            href.push_str("&q=");
            href.push_str(&urlencoding::encode(&find));
        }
        let tab = Tab::new(each.as_query(), href, tr(each.label()));
        match size(rows) {
            Some(number) => tab.count(number, tr(each.label())),
            None => tab,
        }
    };
    let tabs = vec![
        tab(TemplateFamily::Adl14, &adl14),
        tab(TemplateFamily::Adl2, &adl2),
    ];
    let shown = if family == TemplateFamily::Adl2 {
        adl2
    } else {
        adl14
    };
    let list = match shown {
        Ok(rows) if rows.is_empty() => list_empty(
            tr(if find.is_empty() {
                DefinitionsMsg::TemplatesEmpty
            } else {
                DefinitionsMsg::TemplatesNoMatch
            }),
            None,
        ),
        Ok(rows) => rows_table(family, &rows),
        Err(error) => refused(tr(DefinitionsMsg::TemplatesRefused), &error, None),
    };
    let action = "/templates".to_owned();
    let fields = vec![("family".to_owned(), family.as_query().to_owned())];
    let label = tr(DefinitionsMsg::TemplatesFind);
    let toolbar = find_form(
        Find {
            id: "t-find",
            name: "q",
            label: &label,
            value: &find,
            source: StateSource::Url,
            action: Some(&action),
            fields: &fields,
        },
        Html::empty(),
    );
    let chips = (!find.is_empty()).then(|| {
        active_filters(
            &current(),
            ActiveFilters {
                chips: vec![FilterChip {
                    label: tr_with(
                        DefinitionsMsg::ChipContains,
                        &[("value", find.clone().into())],
                    ),
                    remove: format!("/templates?family={}", family.as_query()),
                }],
                clear: format!("/templates?family={}", family.as_query()),
                shown: None,
            },
            None,
        )
    });
    let card = list_card(
        &current(),
        ListCard {
            title: tr(Msg::NavTemplates),
            count: None,
            order: None,
            tabs: Some(TabRow {
                label: tr(DefinitionsMsg::TemplatesFamilies),
                tabs,
                current: family.as_query().to_owned(),
            }),
            actions: None,
            body: list,
            pager: None,
        },
    );
    let body = h(format!(
        r#"<div class="fui-list-page">{}{toolbar}{}{card}</div>"#,
        opt(acted),
        opt(chips)
    ));
    Drawn::page(tr(Msg::NavTemplates), heading.then(body))
}

/// The id of Upload's form.
const UPLOAD_FORM: &str = "template-upload";

/// What Upload read of the post it answered.
#[derive(Debug, Clone)]
pub enum Uploaded {
    /// No upload was posted.
    Fresh,
    /// The CDR refused the upload.
    Refused(ViewerError),
    /// The upload succeeded and the browser was sent on.
    Left,
}

/// Answers an upload posted to Upload a template, of `family`.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
pub async fn load_upload(
    // The family uploaded to.
    family: TemplateFamily,
) -> Result<Uploaded, ViewerError> {
    crate::session::require_session()?;
    let Some(posted) = crate::posted::posted_for("save") else {
        return Ok(Uploaded::Fresh);
    };
    let source = posted.get("source-text").unwrap_or_default();
    let uploaded = match family {
        TemplateFamily::Adl14 => upload_template(source).await.map(|id| template_href(&id)),
        TemplateFamily::Adl2 => upload_adl2_template(source).await.map(|id| {
            let (stem, release) = split_hrid(&id);
            adl2_href(stem, release)
        }),
    };
    Ok(match uploaded {
        Ok(href) => {
            redirect_with(&href, &Done::ok(tr(DefinitionsMsg::TemplateUploaded)));
            Uploaded::Left
        }
        Err(error) => Uploaded::Refused(error),
    })
}

/// Upload a template, of the family the address names.
/// Upload a template.
pub async fn upload_page() -> Drawn {
    let posted = crate::posted::posted();
    let family = posted.as_ref().map_or_else(asked_family, |posted| {
        TemplateFamily::from_query(&posted.text("family"))
    });
    if discarded(posted.as_ref()) {
        crate::request::redirect(&format!("/templates?family={}", family.as_query()));
        return Drawn::default();
    }
    let uploaded = load_upload(family).await;
    let source = posted
        .as_ref()
        .and_then(|posted| posted.get("source-text"))
        .unwrap_or_default();
    let mut top = None;
    let mut line = (Glyph::UPLOAD, tr(DefinitionsMsg::TemplateNotYet));
    match uploaded {
        Ok(Uploaded::Fresh) => {}
        Ok(Uploaded::Left) => return Drawn::default(),
        Ok(Uploaded::Refused(error)) | Err(error) => {
            line = (Glyph::CIRCLE_X, tr(DefinitionsMsg::TemplateNotUploaded));
            let found = findings_of(&error, &source, "source-text")
                .map(|(heading, file)| diagnostics(&current(), &heading, &file));
            top = Some(
                refused(tr(DefinitionsMsg::TemplateUploadRefused), &error, None)
                    .then(opt(found.map(card_panel))),
            );
        }
    }
    dock_editor(EditorDock {
        form: UPLOAD_FORM.to_owned(),
        line: line.1,
        glyph: line.0,
        save: tr(DefinitionsMsg::TemplateUpload),
        blocked: None,
    });
    let families = [TemplateFamily::Adl14, TemplateFamily::Adl2];
    let switch = segmented_links(
        tr(DefinitionsMsg::TemplatesFamily),
        families
            .iter()
            .map(|each| {
                (
                    format!("/templates/upload?family={}", each.as_query()),
                    tr(each.label()),
                )
            })
            .collect(),
        usize::from(family == TemplateFamily::Adl2),
    );
    let adl2 = family == TemplateFamily::Adl2;
    let heading = editor_heading(
        &current(),
        message(DefinitionsMsg::TemplateUploadTitle),
        &Mode::Creating,
        Header {
            title: String::new().into(),
            subtitle: Some(tr(DefinitionsMsg::TemplateUploadSubtitle).into()),
            crumbs: vec![Crumb::new(
                tr(Msg::NavTemplates),
                format!("/templates?family={}", family.as_query()),
            )],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ActionGroup::none(),
    );
    let upload = source_upload(
        &source,
        tr(if adl2 {
            DefinitionsMsg::TemplateAdl2Source
        } else {
            DefinitionsMsg::TemplateOptSource
        }),
        tr(DefinitionsMsg::TemplateChooseFile),
        tr(if adl2 {
            DefinitionsMsg::TemplateAdl2Placeholder
        } else {
            DefinitionsMsg::TemplateOptPlaceholder
        }),
        adl2,
    );
    let form = h(format!(
        r#"<form id="{UPLOAD_FORM}" method="post" action="/templates/upload"><input type="hidden" name="family" value="{}">{}</form>"#,
        family.as_query(),
        card_panel(plain_section(switch).then(plain_section(upload)))
    ));
    Drawn::page(
        tr(DefinitionsMsg::TemplatesUpload),
        heading.then(h(format!(
            r#"<div class="fui-stack">{}{form}</div>"#,
            opt(top)
        ))),
    )
}

/// The tabs of a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateTab {
    /// Its identity and use.
    Overview,
    /// Its path catalog.
    Paths,
    /// Its source.
    Source,
    /// Its operational JSON, ADL 2 only.
    Json,
    /// An example composition.
    Example,
}

impl TemplateTab {
    /// The tab the address names.
    fn asked() -> Self {
        match crate::request::query("tab").as_deref() {
            Some("paths") => Self::Paths,
            Some("source") => Self::Source,
            Some("json") => Self::Json,
            Some("example") => Self::Example,
            _ => Self::Overview,
        }
    }

    /// The tab's key.
    const fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Paths => "paths",
            Self::Source => "source",
            Self::Json => "json",
            Self::Example => "example",
        }
    }

    /// The tab's label.
    fn label(self) -> String {
        match self {
            Self::Overview => tr(Msg::PartyOverview),
            Self::Paths => tr(DefinitionsMsg::TemplatePaths),
            Self::Source => tr(DefinitionsMsg::TemplateSource),
            Self::Json => tr(DefinitionsMsg::TemplateJson),
            Self::Example => tr(DefinitionsMsg::TemplateExample),
        }
    }
}

/// The address of the tab `tab` of the page `base`.
fn tab_of(base: &str, tab: TemplateTab) -> String {
    if tab == TemplateTab::Overview {
        base.to_owned()
    } else {
        format!("{base}?tab={}", tab.key())
    }
}

/// A node of the path catalog and the nodes under it, keyed by place.
fn catalog_node(
    node: &CatalogNode,
    key: &str,
    base: &str,
    selected: Option<&str>,
    open: &mut BTreeSet<String>,
) -> TreeNode {
    if selected.is_some_and(|selected| selected.starts_with(key)) {
        open.insert(key.to_owned());
    }
    let children: Vec<TreeNode> = node
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| catalog_node(child, &format!("{key}-{index}"), base, selected, open))
        .collect();
    let tree = TreeNode::new(key, node.label.clone())
        .tag(node.rm_type.clone())
        .href(format!("{}&node={key}", tab_of(base, TemplateTab::Paths)));
    if children.is_empty() {
        tree
    } else {
        tree.children(children)
    }
}

/// The catalog node `key` names: `n` then a place per level.
fn node_at<'a>(root: &'a CatalogNode, key: &str) -> Option<&'a CatalogNode> {
    let mut parts = key.split('-');
    if parts.next() != Some("n") {
        return None;
    }
    let mut node = root;
    for part in parts {
        node = node.children.get(part.parse::<usize>().ok()?)?;
    }
    Some(node)
}

/// The Paths tab: the catalog tree beside the node it has selected.
///
/// A node links to the query builder only in a COMPOSITION template: the
/// builder's one query shape is `FROM EHR e CONTAINS COMPOSITION c`, which
/// never matches a party of a demographic template.
fn paths_panel(template_id: &str, base: &str, root: &CatalogNode, note: Option<String>) -> Html {
    let selected = crate::request::query("node");
    let mut open = BTreeSet::new();
    open.insert("n".to_owned());
    let tree = catalog_node(root, "n", base, selected.as_deref(), &mut open);
    let left = BrowserPane::new(
        tr(DefinitionsMsg::TemplatePaths),
        tree_view(&Tree {
            id: "template-paths".to_owned(),
            label: tr_with(
                DefinitionsMsg::TemplatePathsOf,
                &[("name", template_id.to_owned().into())],
            ),
            kind: TreeKind::Path,
            editing: false,
            nodes: vec![tree],
            open,
            selected: selected.clone(),
            open_field: None,
            words: TreeWords::of(&current()),
        }),
    );
    let detail_pane = match selected.as_deref().and_then(|key| node_at(root, key)) {
        Some(picked) => path_node(
            &current(),
            PathNode {
                name: picked.label.clone(),
                path: picked.aql_path.clone(),
                rm_type: picked.rm_type.clone(),
                occurrences: String::new(),
                units: (!picked.unit_options.is_empty()).then(|| t(picked.unit_options.join(", "))),
                flat_path: None,
                codes: picked
                    .code_options
                    .iter()
                    .map(|option| {
                        coding(
                            &current(),
                            Coding::new(option.code.clone(), option.label.clone()),
                            Layout::Inline,
                        )
                    })
                    .collect(),
                more: None,
                query: (root.rm_type == "COMPOSITION")
                    .then(|| query_href(template_id, Some(&picked.aql_path))),
            },
        ),
        None => BrowserPane::new(
            tr(DefinitionsMsg::TemplateNode),
            info_note(t(tr(DefinitionsMsg::TemplatePickNode))),
        ),
    };
    panel(
        browser(Panes::Two, Heading::Three, vec![left, detail_pane]).then(opt(note.map(|note| {
            plain_section(h(format!(
                r#"<p class="fui-small fui-muted">{}</p>"#,
                t(note)
            )))
        }))),
    )
}

/// The example options the address names: the format's key, the detail and
/// the type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExampleAsk {
    /// The key of the format asked for.
    format: String,
    /// The detail asked for.
    detail: ExampleDetail,
    /// The type asked for.
    kind: ExampleType,
}

impl ExampleAsk {
    /// The options the address names.
    fn asked() -> Self {
        Self {
            format: crate::request::query("format").unwrap_or_default(),
            detail: match crate::request::query("detail").as_deref() {
                Some("medium") => ExampleDetail::Medium,
                Some("complete") => ExampleDetail::Complete,
                _ => ExampleDetail::Required,
            },
            kind: match crate::request::query("type").as_deref() {
                Some("output") => ExampleType::Output,
                _ => ExampleType::Input,
            },
        }
    }

    /// The format asked for among `formats`, canonical JSON for none.
    fn format(&self, formats: &[(&'static str, ReprFormat)]) -> ReprFormat {
        formats
            .iter()
            .find(|(key, _)| *key == self.format)
            .map_or(ReprFormat::CanonicalJson, |(_, format)| *format)
    }
}

/// The Example tab's options as a `GET` form, `ask` chosen among `formats`.
fn example_options(
    base: &str,
    formats: &[(&'static str, ReprFormat)],
    ask: &ExampleAsk,
) -> (Html, ReprFormat) {
    let format = ask.format(formats);
    let (kind, detail) = (ask.kind, ask.detail);
    let option = |value: &str, words: &str, selected: bool| {
        h(format!(
            r#"<option value="{}"{}>{}</option>"#,
            t(value),
            flag("selected", selected),
            t(words)
        ))
    };
    let format_options: Html = formats
        .iter()
        .map(|(key, each)| option(key, each.label(), *each == format))
        .collect();
    let kind_options: Html = [ExampleType::Input, ExampleType::Output]
        .into_iter()
        .map(|each| option(each.as_query(), &tr(each.label()), each == kind))
        .collect();
    let detail_options: Html = [
        ExampleDetail::Required,
        ExampleDetail::Medium,
        ExampleDetail::Complete,
    ]
    .into_iter()
    .map(|each| option(each.as_query(), &tr(each.label()), each == detail))
    .collect();
    let select = |id: &str, name: &str, options: Html| {
        h(format!(
            r#"<select id="{id}" name="{name}" class="fui-select">{options}</select>"#
        ))
    };
    let form = h(format!(
        r#"<form method="get" action="{}" class="fui-transform-options"><input type="hidden" name="tab" value="example">{}{}{}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-secondary">{}{}</button></div></form>"#,
        t(base),
        Field::new("x-fmt", tr(Msg::CompositionFormat)).view(select(
            "x-fmt",
            "format",
            format_options
        )),
        Field::new("x-typ", tr(DefinitionsMsg::TemplateExampleType)).view(select(
            "x-typ",
            "type",
            kind_options
        )),
        Field::new("x-det", tr(DefinitionsMsg::TemplateExampleDetail)).view(select(
            "x-det",
            "detail",
            detail_options
        )),
        ferrohealth_ui::icon::icon(Glyph::PLAY, "16"),
        t(tr(DefinitionsMsg::TemplateShowExample))
    ));
    (form, format)
}

/// The example the CDR built, pretty-printed on the server, or its refusal.
fn example_panel(
    form: Html,
    format: ReprFormat,
    example: Result<Option<String>, ViewerError>,
) -> Html {
    let shown = match example {
        Ok(Some(text)) => document_section(
            tr_with(
                DefinitionsMsg::TemplateExampleTitle,
                &[("format", format.label().into())],
            ),
            if format == ReprFormat::CanonicalXml {
                Language::Xml
            } else {
                Language::Json
            },
            text,
            "template-example",
        ),
        Ok(None) => plain_section(info_note(t(tr(DefinitionsMsg::TemplateNoExample)))),
        Err(error) => plain_section(refused(
            tr(DefinitionsMsg::TemplateExampleRefused),
            &error,
            None,
        )),
    };
    panel(plain_section(form).then(shown))
}

/// The header of a template: its name, its family line, Build a query (for a
/// template the query builder reaches, `queryable`), the More menu with the
/// template's `download` address, and what comes `after`.
fn template_header(
    title: String,
    subtitle: String,
    template_id: &str,
    family: TemplateFamily,
    download: String,
    after: Option<Html>,
    queryable: bool,
) -> Html {
    let more = Menu::new(
        "template-more",
        tr(DefinitionsMsg::TemplateMore),
        vec![
            Item::new(Action::new(
                Some(Glyph::DOWNLOAD),
                tr(DefinitionsMsg::TemplateDownload),
                Act::Download(download),
            )),
            Item::new(Action::new(
                Some(Glyph::COPY),
                tr(DefinitionsMsg::TemplateCopyId),
                Act::Copy(template_id.to_owned()),
            )),
        ],
    )
    .destructive(vec![Item::new(Action::new(
        Some(Glyph::TRASH),
        tr(DefinitionsMsg::TemplateDelete),
        Act::Dialog(DELETE_DIALOG.to_owned()),
    ))]);
    let heading = page_heading(
        &current(),
        Header {
            title: title.into(),
            subtitle: Some(subtitle.into()),
            crumbs: vec![Crumb::new(
                tr(Msg::NavTemplates),
                format!("/templates?family={}", family.as_query()),
            )],
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        if queryable {
            ActionGroup::primary(Action::new(
                Some(Glyph::SQUARE_TERMINAL),
                tr(DefinitionsMsg::TemplateBuildQuery),
                Act::Link(query_href(template_id, None)),
            ))
        } else {
            ActionGroup::none()
        }
        .more(more),
    );
    heading.then(opt(after))
}

/// What one ADL 1.4 template's page read.
#[derive(Debug, Clone)]
pub struct Template {
    /// The CDR's refusal of the delete posted.
    acted: Option<ViewerError>,
    /// The template, carrying only the parts its tab draws.
    detail: Result<TemplateDetail, ViewerError>,
    /// The template's download address.
    download: String,
    /// When it was uploaded, on the Overview tab.
    uploaded: Option<String>,
    /// The example the CDR built, on the Example tab of a root it builds
    /// examples for.
    example: Option<Result<Option<String>, ViewerError>>,
}

/// Answers a delete posted to the ADL 1.4 template `template_id` and reads
/// what its tab `tab` draws, the example as `ask` names it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
pub async fn load_template(
    // The template's id.
    template_id: String,
    // The tab shown.
    tab: TemplateTab,
    // The example options the address names.
    ask: ExampleAsk,
) -> Result<Template, ViewerError> {
    crate::session::require_session()?;
    let acted = delete_posted("/templates").await;
    let download = crate::download::template_href(TemplateFamily::Adl14, &template_id);
    let mut detail = fetch_template_detail(template_id.clone()).await;
    let mut uploaded = None;
    let mut example = None;
    match &mut detail {
        Err(error) => {
            if crate::pages::common::is_status(error, http::StatusCode::NOT_FOUND) {
                crate::request::set_status(http::StatusCode::NOT_FOUND);
            }
        }
        Ok(detail) => {
            // NOTE: no openEHR spec governs this: our own design; the model keeps
            // only what the tab draws, so the page carries no unshown source.
            if tab != TemplateTab::Source {
                detail.source = String::new();
            }
            if tab != TemplateTab::Paths {
                detail.catalog.children = Vec::new();
            }
            match tab {
                TemplateTab::Example => {
                    let formats = crate::example_options::example_formats(&detail.catalog.rm_type);
                    if !formats.is_empty() {
                        let format = ask.format(formats);
                        example = Some(
                            fetch_example(template_id.clone(), format, ask.detail, ask.kind)
                                .await
                                .map(|text| Some(crate::bff::pretty_body(&text, format))),
                        );
                    }
                }
                TemplateTab::Overview => {
                    uploaded = list_templates()
                        .await
                        .ok()
                        .and_then(|rows| {
                            rows.into_iter().find(|row| row.template_id == template_id)
                        })
                        .map(|row| row.created);
                }
                TemplateTab::Paths | TemplateTab::Source | TemplateTab::Json => {}
            }
        }
    }
    Ok(Template {
        acted,
        detail,
        download,
        uploaded,
        example,
    })
}

/// One ADL 1.4 template.
/// One ADL 1.4 template.
#[expect(
    clippy::too_many_lines,
    reason = "one page: the model it reads and the view it draws"
)]
pub async fn page() -> Drawn {
    let template_id = crate::routes::params()
        .get("template_id")
        .unwrap_or_default();
    let tab = match TemplateTab::asked() {
        TemplateTab::Json => TemplateTab::Overview,
        tab => tab,
    };
    let ask = ExampleAsk::asked();
    let base = template_href(&template_id);
    let Template {
        acted,
        detail,
        download,
        uploaded,
        example,
    } = match load_template(template_id.clone(), tab, ask.clone()).await {
        Ok(template) => template,
        Err(error) => {
            return Drawn::page(
                template_id,
                refused(tr(DefinitionsMsg::TemplateRefused), &error, None),
            );
        }
    };
    let acted = delete_refused(acted.as_ref());
    let detail: TemplateDetail = match detail {
        Ok(detail) => detail,
        Err(error) => {
            let body = Html::join([
                template_header(
                    template_id.clone(),
                    tr(DefinitionsMsg::TemplateAdl14Line),
                    &template_id,
                    TemplateFamily::Adl14,
                    download,
                    None,
                    true,
                ),
                opt(acted),
                refused(tr(DefinitionsMsg::TemplateRefused), &error, None),
            ]);
            return Drawn::page(template_id, body);
        }
    };
    let queryable = detail.catalog.rm_type == "COMPOSITION";
    let tabs: Vec<Tab> = [
        TemplateTab::Overview,
        TemplateTab::Paths,
        TemplateTab::Source,
        TemplateTab::Example,
    ]
    .into_iter()
    .map(|each| Tab::new(each.key(), tab_of(&base, each), each.label()))
    .collect();
    let body = match tab {
        TemplateTab::Paths => paths_panel(&template_id, &base, &detail.catalog, None),
        TemplateTab::Source => panel(document_section(
            tr(DefinitionsMsg::TemplateSourceTitle),
            Language::Xml,
            detail.source.clone(),
            "template-source",
        )),
        TemplateTab::Example => {
            let root = detail.catalog.rm_type.clone();
            let formats = crate::example_options::example_formats(&root);
            match example {
                Some(example) if !formats.is_empty() => {
                    let (form, format) = example_options(&base, formats, &ask);
                    example_panel(form, format, example)
                }
                _ => panel(plain_section(info_note(t(tr_with(
                    DefinitionsMsg::TemplateNoExampleForRoot,
                    &[("root", root.into())],
                ))))),
            }
        }
        _ => {
            let meta = &detail.meta;
            let mut facts = vec![
                (tr(DefinitionsMsg::TemplateConcept), t(&meta.concept)),
                (tr(DefinitionsMsg::TemplateVersion), t(&meta.version)),
                (
                    tr(DefinitionsMsg::TemplateDefaultLanguage),
                    t(&meta.language),
                ),
                (
                    tr(DefinitionsMsg::TemplateLanguages),
                    t(meta.languages.join(", ")),
                ),
                (
                    tr(DefinitionsMsg::TemplateUid),
                    h(format!("<code>{}</code>", t(&meta.uid))),
                ),
                (
                    tr(DefinitionsMsg::ColumnRootArchetype),
                    h(format!("<code>{}</code>", t(&detail.catalog.node_id))),
                ),
            ];
            if let Some(uploaded) = uploaded {
                facts.push((tr(DefinitionsMsg::ColumnUploaded), when(&uploaded)));
            }
            let use_section = queryable.then(|| {
                section(
                    Some(tr(DefinitionsMsg::TemplateUse)),
                    None,
                    None,
                    h(format!(
                        r#"<p><a href="{}">{}</a></p>"#,
                        t(query_href(&template_id, None)),
                        t(tr(DefinitionsMsg::TemplateUseLine))
                    )),
                )
            });
            panel(
                section(
                    Some(tr(DefinitionsMsg::TemplateIdentity)),
                    None,
                    None,
                    definitions(facts),
                )
                .then(opt(use_section)),
            )
        }
    };
    let title = if detail.meta.template_id.is_empty() {
        template_id.clone()
    } else {
        detail.meta.template_id.clone()
    };
    let binder = binder(
        &current(),
        tr(DefinitionsMsg::TemplateViews),
        tabs,
        tab.key(),
        body,
    );
    let page = Html::join([
        template_header(
            title.clone(),
            tr_with(
                DefinitionsMsg::TemplateAdl14Subtitle,
                &[("concept", detail.meta.concept.clone().into())],
            ),
            &template_id,
            TemplateFamily::Adl14,
            download,
            None,
            queryable,
        ),
        h(format!(
            r#"<div class="fui-stack">{}{}{binder}</div>"#,
            opt(acted),
            opt(inline_confirm(TemplateFamily::Adl14, &template_id, &base))
        )),
        delete_dialog(TemplateFamily::Adl14, &template_id, DELETE_DIALOG, &base),
    ]);
    Drawn::page(title, page)
}

/// What the tab of an ADL 2 template read.
#[derive(Debug, Clone)]
pub enum Adl2Body {
    /// Its identity, from the list.
    Overview,
    /// Its path catalog.
    Paths(Result<Option<CatalogNode>, ViewerError>),
    /// Its source.
    Source(Result<Option<String>, ViewerError>),
    /// Its operational JSON, pretty-printed.
    Json(Result<Option<String>, ViewerError>),
    /// The example the CDR built, pretty-printed.
    Example(Result<Option<String>, ViewerError>),
}

/// What one ADL 2 template's page read.
#[derive(Debug, Clone)]
pub struct Adl2Template {
    /// The CDR's refusal of the delete posted.
    acted: Option<ViewerError>,
    /// The HRID without its release.
    stem: String,
    /// Every release the CDR holds of the template, oldest first.
    releases: Vec<String>,
    /// The latest release.
    latest: Option<String>,
    /// The release read.
    release: Option<String>,
    /// The HRID read.
    hrid: String,
    /// The template's row in the list.
    row: Option<TemplateRow>,
    /// The tab shown.
    tab: TemplateTab,
    /// What the tab read.
    body: Adl2Body,
    /// The template's download address.
    download: String,
}

/// What an ADL 2 template's page loads: the page, or nothing for a post or
/// a release choice it answered by sending the browser on.
#[derive(Debug, Clone)]
pub enum Adl2Page {
    /// The page.
    Read(Box<Adl2Template>),
    /// The browser was sent on.
    Left,
}

/// The tabs an ADL 2 template offers: Overview alone for an older release.
fn adl2_tabs(older: bool) -> Vec<TemplateTab> {
    let mut tabs = vec![TemplateTab::Overview];
    if !older {
        tabs.extend([
            TemplateTab::Paths,
            TemplateTab::Source,
            TemplateTab::Json,
            TemplateTab::Example,
        ]);
    }
    tabs
}

/// Answers a delete posted to the ADL 2 template `asked_id`, sends a release
/// `chosen` in the form on to its page, and reads the release
/// `asked_release` names, the latest for none, with what its tab `tab`
/// draws.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session.
pub async fn load_adl2_template(
    // The template id the address names.
    asked_id: String,
    // The release the address names.
    asked_release: Option<String>,
    // The release chosen in the release form.
    chosen: Option<String>,
    // The tab the address names.
    tab: TemplateTab,
    // The example options the address names.
    ask: ExampleAsk,
) -> Result<Adl2Page, ViewerError> {
    crate::session::require_session()?;
    let acted = delete_posted("/templates?family=adl2").await;
    let (stem, named) = split_hrid(&asked_id);
    let stem = stem.to_owned();
    let listed = list_adl2_templates().await;
    let ids: Vec<String> = listed
        .as_ref()
        .map(|rows| rows.iter().map(|row| row.template_id.clone()).collect())
        .unwrap_or_default();
    let releases = family_versions(&ids, &stem);
    let latest = releases.last().cloned();
    let release = asked_release
        .clone()
        .or_else(|| named.map(str::to_owned))
        .or_else(|| latest.clone());
    let hrid = release
        .as_ref()
        .map_or_else(|| stem.clone(), |release| format!("{stem}.v{release}"));
    if let Some(chosen) = chosen {
        crate::request::redirect(&adl2_href(
            &stem,
            (Some(&chosen) != latest.as_ref()).then_some(chosen.as_str()),
        ));
        return Ok(Adl2Page::Left);
    }
    let row = listed
        .as_ref()
        .ok()
        .and_then(|rows| rows.iter().find(|row| row.template_id == hrid).cloned());
    let older = release.is_some() && release != latest;
    let tab = if adl2_tabs(older).contains(&tab) {
        tab
    } else {
        TemplateTab::Overview
    };
    let version = release.clone();
    let body = match tab {
        TemplateTab::Paths => Adl2Body::Paths(fetch_adl2_catalog(hrid.clone(), version).await),
        TemplateTab::Source => Adl2Body::Source(fetch_adl2_source(hrid.clone(), version).await),
        TemplateTab::Json => {
            Adl2Body::Json(fetch_adl2_json(hrid.clone(), version).await.map(|json| {
                json.map(|json| crate::bff::pretty_body(&json, ReprFormat::CanonicalJson))
            }))
        }
        TemplateTab::Example => {
            let format = ask.format(crate::example_options::CANONICAL_EXAMPLE_FORMATS);
            Adl2Body::Example(
                fetch_adl2_example(hrid.clone(), format, ask.detail, ask.kind)
                    .await
                    .map(|text| text.map(|text| crate::bff::pretty_body(&text, format))),
            )
        }
        TemplateTab::Overview => Adl2Body::Overview,
    };
    let download = crate::download::template_href(TemplateFamily::Adl2, &hrid);
    Ok(Adl2Page::Read(Box::new(Adl2Template {
        acted,
        stem,
        releases,
        latest,
        release,
        hrid,
        row,
        tab,
        body,
        download,
    })))
}

/// One ADL 2 template, at the release the address names or the latest.
/// One ADL 2 template, at its latest release or the one asked for.
#[expect(
    clippy::too_many_lines,
    reason = "one page: the model it reads and the view it draws"
)]
pub async fn adl2_page() -> Drawn {
    let params = crate::routes::params();
    let (asked_id, asked_release) = (
        params.get("template_id").unwrap_or_default(),
        params.get("release"),
    );
    let ask = ExampleAsk::asked();
    let template = match load_adl2_template(
        asked_id.clone(),
        asked_release.clone(),
        crate::request::query("release"),
        TemplateTab::asked(),
        ask.clone(),
    )
    .await
    {
        Ok(Adl2Page::Read(template)) => *template,
        Ok(Adl2Page::Left) => return Drawn::default(),
        Err(error) => {
            return Drawn::page(
                asked_id,
                refused(tr(DefinitionsMsg::TemplateRefused), &error, None),
            );
        }
    };
    let Adl2Template {
        acted,
        stem,
        releases,
        latest,
        release,
        hrid,
        row,
        tab,
        body,
        download,
    } = template;
    let acted = delete_refused(acted.as_ref());
    let base = adl2_href(&stem, asked_release.as_deref());
    let older = release.is_some() && release != latest;
    let after = if older {
        Some(info_note(h(format!(
            r#"{} <a href="{}">{}</a>"#,
            t(tr_with(
                DefinitionsMsg::TemplateReadingRelease,
                &[("release", release.clone().unwrap_or_default().into())],
            )),
            t(adl2_href(&stem, None)),
            t(tr_with(
                DefinitionsMsg::TemplateReadLatest,
                &[("release", latest.clone().unwrap_or_default().into())],
            ))
        ))))
    } else if releases.len() > 1 {
        let options: Html = releases
            .iter()
            .rev()
            .map(|each| {
                h(format!(
                    r#"<option value="{}"{}>{}</option>"#,
                    t(each),
                    flag("selected", Some(each) == release.as_ref()),
                    t(each)
                ))
            })
            .collect();
        Some(h(format!(
            r#"<form class="fui-cluster fui-title-select" method="get" action="{}"><label class="fui-label" for="ver">{}</label><select id="ver" name="release" class="fui-select">{options}</select><button type="submit" class="fui-button fui-button-secondary fui-button-small">{}</button></form>"#,
            t(adl2_href(&stem, None)),
            t(tr(DefinitionsMsg::TemplateRelease)),
            t(tr(Msg::PartiesOpen))
        )))
    } else {
        None
    };
    let title = row
        .as_ref()
        .map_or_else(|| stem.clone(), |row| row.concept.clone());
    let tab_list: Vec<Tab> = adl2_tabs(older)
        .iter()
        .map(|each| Tab::new(each.key(), tab_of(&base, *each), each.label()))
        .collect();
    let refusal = |error: &ViewerError| {
        panel(plain_section(refused(
            tr(DefinitionsMsg::TemplateRefused),
            error,
            None,
        )))
    };
    let body = match body {
        Adl2Body::Paths(catalog) => match catalog {
            Ok(Some(root)) => paths_panel(
                &hrid,
                &base,
                &root,
                Some(tr(DefinitionsMsg::TemplateAdl2Paths)),
            ),
            Ok(None) => panel(plain_section(info_note(t(tr(
                DefinitionsMsg::TemplateNoCatalog,
            ))))),
            Err(error) => refusal(&error),
        },
        Adl2Body::Source(source) => match source {
            Ok(Some(source)) => panel(document_section(
                tr(DefinitionsMsg::TemplateSourceTitle),
                Language::Plain,
                source,
                "template-source",
            )),
            Ok(None) => panel(plain_section(info_note(t(tr(
                DefinitionsMsg::TemplateNoSource,
            ))))),
            Err(error) => refusal(&error),
        },
        Adl2Body::Json(json) => match json {
            Ok(Some(json)) => panel(document_section(
                tr_with(
                    DefinitionsMsg::TemplateJsonTitle,
                    &[("release", release.clone().unwrap_or_default().into())],
                ),
                Language::Json,
                json,
                "template-json",
            )),
            Ok(None) => panel(plain_section(info_note(t(tr(
                DefinitionsMsg::TemplateNoSource,
            ))))),
            Err(error) => refusal(&error),
        },
        Adl2Body::Example(example) => {
            let (form, format) = example_options(
                &base,
                crate::example_options::CANONICAL_EXAMPLE_FORMATS,
                &ask,
            );
            example_panel(form, format, example)
        }
        Adl2Body::Overview => {
            let mut facts = vec![(tr(DefinitionsMsg::TemplateConcept), t(&title))];
            if let Some(release) = &release {
                facts.push((tr(DefinitionsMsg::TemplateRelease), t(release)));
            }
            if let Some(row) = &row {
                facts.push((
                    tr(DefinitionsMsg::ColumnRootArchetype),
                    h(format!("<code>{}</code>", t(&row.archetype_id))),
                ));
                facts.push((tr(DefinitionsMsg::ColumnUploaded), when(&row.created)));
            }
            facts.push((
                tr(DefinitionsMsg::TemplateReleases),
                t(count_words(releases.len())),
            ));
            panel(
                section(
                    Some(tr(DefinitionsMsg::TemplateIdentity)),
                    None,
                    None,
                    definitions(facts),
                )
                .then(section(
                    Some(tr(DefinitionsMsg::TemplateUse)),
                    None,
                    None,
                    h(format!(
                        r#"<p><a href="{}">{}</a></p>"#,
                        t(query_href(&hrid, None)),
                        t(tr(DefinitionsMsg::TemplateUseLine))
                    )),
                )),
            )
        }
    };
    let subtitle = tr_with(
        DefinitionsMsg::TemplateAdl2Subtitle,
        &[("hrid", hrid.clone().into())],
    );
    let binder = binder(
        &current(),
        tr(DefinitionsMsg::TemplateViews),
        tab_list,
        tab.key(),
        body,
    );
    let page = Html::join([
        template_header(
            title.clone(),
            subtitle,
            &hrid,
            TemplateFamily::Adl2,
            download,
            after,
            true,
        ),
        h(format!(
            r#"<div class="fui-stack">{}{}{binder}</div>"#,
            opt(acted),
            opt(inline_confirm(TemplateFamily::Adl2, &hrid, &base))
        )),
        delete_dialog(TemplateFamily::Adl2, &hrid, DELETE_DIALOG, &base),
    ]);
    Drawn::page(title, page)
}

/// The words counting the releases of an ADL 2 template.
fn count_words(total: usize) -> String {
    tr_with(
        DefinitionsMsg::TemplateReleaseCount,
        &[("count", count(total))],
    )
}
