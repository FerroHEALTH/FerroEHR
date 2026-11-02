// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The releases of a template on its pages.
//!
//! The list draws one row per lineage with its other releases under it; the
//! template's header names its release with every other release one link
//! away (FerroUI R29); the Overview lists the releases with the compositions
//! committed against each; Compare reads what changed in the paths between
//! two (FerroUI's change table). A release is the template id's
//! `.v{version}` (BASE `master05-identification_package.adoc` §Template
//! Identifiers); the lineage is [`crate::template_releases`]. No openEHR
//! spec governs these pages: our own design.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::i18n::Word;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::table::view::{
    Body, Column, DataTable, Role, Sorting, cell, data_table, menu_cell, title_cell,
};
use ferrohealth_ui::view::action_menu::{
    Act, Action, Choice, Item, Listbox, Menu, Pick, listbox, row_menu,
};
use ferrohealth_ui::view::change::change_table;
use ferrohealth_ui::view::form::Field;
use ferrohealth_ui::view::tag::Tag;

use crate::adl2::{TemplateFamily, split_hrid};
use crate::bff::dashboard::Count;
use crate::bff::template_adl2::fetch_adl2_catalog;
use crate::bff::template_detail::fetch_template_catalog;
use crate::bff::templates::{TemplateRow, count_compositions};
use crate::builder::catalog::CatalogNode;
use crate::error::ViewerError;
use crate::locale::definitions::DefinitionsMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{flag, h, t};
use crate::pages::common::{ago, count, info_note, panel, plain_section, refused, section};
use crate::pages::templates::{TemplateTab, adl2_href, query_href, template_href};
use crate::template_releases::{Comparison, Lineage, compare, lineage_of, release_label};

/// The page of the release `row` of a template of `family`.
#[must_use]
pub fn release_href(family: TemplateFamily, row: &TemplateRow) -> String {
    match family {
        TemplateFamily::Adl14 => template_href(&row.template_id),
        TemplateFamily::Adl2 => {
            let (stem, release) = split_hrid(&row.template_id);
            adl2_href(stem, release)
        }
    }
}

/// The name of a release in a list: an ADL 2 identifier in the code face.
fn release_name(family: TemplateFamily, row: &TemplateRow) -> Html {
    match family {
        TemplateFamily::Adl14 => t(&row.template_id),
        TemplateFamily::Adl2 => h(format!("<code>{}</code>", t(&row.template_id))),
    }
}

/// The row menu of the release `row`, the `index`th row of the list.
fn release_menu(index: usize, href: &str, row: &TemplateRow) -> Html {
    let confirm = format!(
        "{href}{}confirm=delete",
        if href.contains('?') { '&' } else { '?' }
    );
    row_menu(
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
    )
}

/// The templates of one family as a table: a body per lineage, its newest
/// release first with the count of releases, the earlier releases under it.
#[must_use]
pub fn lineage_table(family: TemplateFamily, lineages: &[Lineage]) -> Html {
    let columns = [
        Column::text("template", tr(DefinitionsMsg::ColumnTemplate)).role(Role::Title),
        Column::text("root", tr(DefinitionsMsg::ColumnRootArchetype)).role(Role::Sub),
        Column::text("releases", tr(DefinitionsMsg::TemplateReleases)).role(Role::Meta),
        Column::text("uploaded", tr(DefinitionsMsg::ColumnUploaded)).role(Role::Meta),
        Column::text("menu", tr(Msg::ColumnActions)).role(Role::Menu),
    ];
    let mut index = 0_usize;
    let bodies: Html = lineages
        .iter()
        .map(|lineage| {
            let rows: Html = lineage
                .releases()
                .iter()
                .enumerate()
                .map(|(place, row)| {
                    let href = release_href(family, row);
                    let (sub, releases) = if place == 0 {
                        (
                            t(&row.concept),
                            t(tr_with(
                                DefinitionsMsg::TemplateReleaseCount,
                                &[("count", count(lineage.releases().len()))],
                            )),
                        )
                    } else {
                        (t(tr(DefinitionsMsg::TemplateEarlierRelease)), Html::empty())
                    };
                    let drawn = h(format!(
                        "<tr>{}{}{}{}{}</tr>",
                        title_cell(Some(&href), release_name(family, row), Some(sub)),
                        cell(
                            &columns[1],
                            h(format!("<code>{}</code>", t(&row.archetype_id)))
                        ),
                        cell(&columns[2], releases),
                        cell(&columns[3], ago(&row.created)),
                        menu_cell(release_menu(index, &href, row))
                    ));
                    index = index.saturating_add(1);
                    drawn
                })
                .collect();
            h("<tbody>".to_owned())
                .then(rows)
                .then(h("</tbody>".to_owned()))
        })
        .collect();
    let caption = tr(family.label());
    data_table(DataTable {
        caption: &caption,
        columns: &columns,
        sorting: Sorting::default(),
        body: Body::Bodies(bodies),
    })
}

/// The two releases a comparison asks for, by their labels.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CompareAsk {
    /// The earlier side.
    from: Option<String>,
    /// The later side.
    to: Option<String>,
}

impl CompareAsk {
    /// The releases the address names.
    #[must_use]
    pub fn asked() -> Self {
        let named = |name| crate::request::query(name).filter(|value| !value.trim().is_empty());
        Self {
            from: named("from"),
            to: named("to"),
        }
    }
}

/// What a comparison read: its two releases and the change between them.
#[derive(Debug, Clone)]
pub struct Compared {
    /// The release compared from.
    from: TemplateRow,
    /// The release compared to.
    to: TemplateRow,
    /// The change, `None` when the CDR holds no template for a side.
    change: Result<Option<Comparison>, ViewerError>,
}

/// The compositions committed against each release, in the lineage's order,
/// or the refusal of the session.
type Counts = Result<Vec<Result<Count, ViewerError>>, ViewerError>;

/// What the releases of a template read for its page.
#[derive(Debug, Clone, Default)]
pub struct Releases {
    /// The CDR's refusal of the list the releases are read from.
    unlisted: Option<ViewerError>,
    /// The lineage the template belongs to, when the CDR lists it.
    lineage: Option<Lineage>,
    /// The compositions committed against each release, on the Overview.
    counts: Option<Counts>,
    /// The comparison, on the Compare tab.
    compared: Option<Compared>,
}

/// The path catalog of the release `row` of `family`, `None` when the CDR
/// holds no template for it.
async fn catalog(
    family: TemplateFamily,
    row: &TemplateRow,
) -> Result<Option<CatalogNode>, ViewerError> {
    match family {
        TemplateFamily::Adl14 => fetch_template_catalog(row.template_id.clone())
            .await
            .map(Some),
        TemplateFamily::Adl2 => {
            let release = split_hrid(&row.template_id).1.map(str::to_owned);
            fetch_adl2_catalog(row.template_id.clone(), release).await
        }
    }
}

/// Reads the releases of the template `template_id` of `family` among the
/// listed `rows`, with what the tab `tab` draws of them: the composition
/// counts on the Overview, the comparison `ask` names on Compare.
pub async fn read_releases(
    // The family of the template.
    family: TemplateFamily,
    // The templates the CDR lists of that family, or its refusal.
    rows: Result<Vec<TemplateRow>, ViewerError>,
    // The release the page shows.
    template_id: &str,
    // The tab shown.
    tab: TemplateTab,
    // The comparison the address names.
    ask: CompareAsk,
) -> Releases {
    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => {
            return Releases {
                unlisted: Some(error),
                ..Releases::default()
            };
        }
    };
    let Some(lineage) = lineage_of(rows, template_id) else {
        return Releases::default();
    };
    let counts = match tab {
        TemplateTab::Overview => Some(
            count_compositions(
                lineage
                    .releases()
                    .iter()
                    .map(|row| row.template_id.clone())
                    .collect(),
            )
            .await,
        ),
        _ => None,
    };
    let mut compared = None;
    if tab == TemplateTab::Compare {
        let side = |label: Option<&String>| {
            label.and_then(|label| lineage.labelled(family, label).cloned())
        };
        let to = side(ask.to.as_ref()).or_else(|| lineage.release(template_id).cloned());
        let from = side(ask.from.as_ref()).or_else(|| {
            to.as_ref()
                .and_then(|to| lineage.neighbour(&to.template_id).cloned())
        });
        if let (Some(from), Some(to)) = (from, to) {
            let change = if from.template_id == to.template_id {
                Ok(Some(Comparison::default()))
            } else {
                match (catalog(family, &from).await, catalog(family, &to).await) {
                    (Ok(Some(was)), Ok(Some(is))) => Ok(Some(compare(&was, &is))),
                    (Err(error), _) | (_, Err(error)) => Err(error),
                    _ => Ok(None),
                }
            };
            compared = Some(Compared { from, to, change });
        }
    }
    Releases {
        unlisted: None,
        lineage: Some(lineage),
        counts,
        compared,
    }
}

impl Releases {
    /// Whether the template has more than one release.
    #[must_use]
    pub fn several(&self) -> bool {
        self.lineage
            .as_ref()
            .is_some_and(|lineage| lineage.releases().len() > 1)
    }

    /// The release `template_id` names, for its upload time.
    #[must_use]
    pub fn row(&self, template_id: &str) -> Option<&TemplateRow> {
        self.lineage.as_ref()?.release(template_id)
    }

    /// The release named beside the template's title: the select button of
    /// every release, each a link and the newest marked, or the one release
    /// as a tag.
    #[must_use]
    pub fn badge(&self, family: TemplateFamily, template_id: &str) -> Option<Html> {
        let lineage = self.lineage.as_ref()?;
        let shown = lineage.release(template_id)?;
        let label = release_label(family, shown);
        if lineage.releases().len() < 2 {
            return (family == TemplateFamily::Adl2).then(|| {
                Tag::new(label)
                    .mono()
                    .title(tr(DefinitionsMsg::TemplateRelease))
                    .view()
            });
        }
        let choices = lineage
            .releases()
            .iter()
            .map(|row| Choice {
                label: Word::from(release_label(family, row)),
                note: lineage
                    .is_newest(&row.template_id)
                    .then(|| Word::from(tr(DefinitionsMsg::TemplateNewest))),
                lang: None,
                selected: row.template_id.eq_ignore_ascii_case(&shown.template_id),
                pick: Pick::Link(release_href(family, row)),
            })
            .collect();
        Some(listbox(
            &current(),
            &Listbox {
                id: "template-releases".to_owned(),
                label: tr(DefinitionsMsg::TemplateRelease),
                value: Word::from(label),
                choices,
            },
        ))
    }

    /// The Overview's releases: each a link with its upload time and the
    /// compositions committed against it, the newest and the one shown
    /// marked.
    #[must_use]
    pub fn overview(&self, family: TemplateFamily, template_id: &str) -> Option<Html> {
        let refusal = |error: &ViewerError| {
            section(
                Some(tr(DefinitionsMsg::TemplateReleases)),
                None,
                None,
                refused(tr(DefinitionsMsg::TemplateCountsRefused), error, None),
            )
        };
        if let Some(error) = &self.unlisted {
            return Some(refusal(error));
        }
        let lineage = self.lineage.as_ref()?;
        let columns = [
            Column::text("release", tr(DefinitionsMsg::TemplateRelease)).role(Role::Title),
            Column::text("uploaded", tr(DefinitionsMsg::ColumnUploaded)).role(Role::Meta),
            Column::text("compositions", tr(DefinitionsMsg::TemplateCompositions))
                .numeric()
                .role(Role::Meta),
        ];
        let counts = match &self.counts {
            Some(Err(error)) => return Some(refusal(error)),
            Some(Ok(counts)) => counts.as_slice(),
            None => &[],
        };
        let rows: Html = lineage
            .releases()
            .iter()
            .enumerate()
            .map(|(place, row)| {
                let mut marks = Vec::new();
                if place == 0 {
                    marks.push(tr(DefinitionsMsg::TemplateNewest));
                }
                if row.template_id.eq_ignore_ascii_case(template_id) {
                    marks.push(tr(DefinitionsMsg::TemplateShownHere));
                }
                let compositions = match counts.get(place) {
                    Some(Ok(Ok(number))) => {
                        t(ferrohealth_ui::i18n::number::grouped(&current(), *number))
                    }
                    Some(Ok(Err(shape))) => crate::pages::dashboard::unavailable(*shape),
                    Some(Err(_)) => t(tr(DefinitionsMsg::TemplateCountRefused)),
                    None => t(tr(Msg::NoValue)),
                };
                h(format!(
                    "<tr>{}{}{}</tr>",
                    title_cell(
                        Some(&release_href(family, row)),
                        t(release_label(family, row)),
                        (!marks.is_empty()).then(|| t(marks.join(" · ")))
                    ),
                    cell(&columns[1], ago(&row.created)),
                    cell(&columns[2], compositions)
                ))
            })
            .collect();
        let caption = tr(DefinitionsMsg::TemplateReleases);
        Some(section(
            Some(caption.clone()),
            Some(u64::try_from(lineage.releases().len()).unwrap_or(u64::MAX)),
            None,
            data_table(DataTable {
                caption: &caption,
                columns: &columns,
                sorting: Sorting::default(),
                body: Body::Rows(rows),
            }),
        ))
    }

    /// The Compare tab: the choice of two releases as a `GET` form to
    /// `base`, and the paths the later adds, removes and changes.
    #[must_use]
    pub fn compare_panel(&self, family: TemplateFamily, base: &str) -> Html {
        let (Some(lineage), Some(compared)) = (self.lineage.as_ref(), self.compared.as_ref())
        else {
            return panel(plain_section(info_note(t(tr(
                DefinitionsMsg::TemplateCompareNone,
            )))));
        };
        let select = |id: &str, name: &str, chosen: &TemplateRow| {
            let options: Html = lineage
                .releases()
                .iter()
                .map(|row| {
                    let label = release_label(family, row);
                    h(format!(
                        r#"<option value="{}"{}>{}</option>"#,
                        t(&label),
                        flag("selected", row.template_id == chosen.template_id),
                        t(&label)
                    ))
                })
                .collect();
            h(format!(
                r#"<select id="{id}" name="{name}" class="fui-select">{options}</select>"#
            ))
        };
        let form = h(format!(
            r#"<form method="get" action="{}" class="fui-transform-options"><input type="hidden" name="tab" value="compare">{}{}<div class="fui-transform-run"><button type="submit" class="fui-button fui-button-secondary">{}</button></div></form>"#,
            t(base),
            Field::new("cmp-from", tr(DefinitionsMsg::TemplateCompareFrom)).view(select(
                "cmp-from",
                "from",
                &compared.from
            )),
            Field::new("cmp-to", tr(DefinitionsMsg::TemplateCompareTo)).view(select(
                "cmp-to",
                "to",
                &compared.to
            )),
            t(tr(DefinitionsMsg::TemplateCompare))
        ));
        let (from, to) = (
            release_label(family, &compared.from),
            release_label(family, &compared.to),
        );
        let shown = match &compared.change {
            Ok(Some(_)) if compared.from.template_id == compared.to.template_id => {
                plain_section(info_note(t(tr(DefinitionsMsg::TemplateCompareSame))))
            }
            Ok(Some(change)) => section(
                Some(tr_with(
                    DefinitionsMsg::TemplateCompareSummary,
                    &[
                        ("added", count(change.added)),
                        ("removed", count(change.removed)),
                        ("changed", count(change.changed)),
                    ],
                )),
                None,
                None,
                change_table(
                    &current(),
                    tr_with(
                        DefinitionsMsg::TemplateCompareCaption,
                        &[("from", from.into()), ("to", to.into())],
                    ),
                    &change.changes,
                ),
            ),
            Ok(None) => plain_section(info_note(t(tr(DefinitionsMsg::TemplateNoCatalog)))),
            Err(error) => plain_section(refused(
                tr(DefinitionsMsg::TemplateCompareRefused),
                error,
                None,
            )),
        };
        panel(plain_section(form).then(shown))
    }
}
