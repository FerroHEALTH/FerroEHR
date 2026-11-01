// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The top bar's search and the find page: what the CDR holds under the words
//! typed, and where an id can be opened.
//!
//! The search posts its words, since an id can name a patient's record, so
//! they never reach a URL (FerroUI `console-ui.md` §5). An id shaped as a UUID
//! is read as an EHR id and as a composition's versioned object, and is tried
//! as a party; any words are matched against the template ids and concepts
//! and the stored queries' names. A subject id needs its namespace, which the
//! EHRs list asks for. As you type, the panel under the search box asks the
//! answer route, which answers with [`find_answer`], and shows the hits
//! grouped by kind; the
//! find page lists every hit in the page and the other ways to read the id.
//! No openEHR spec governs the viewer: our own design; the reads are
//! ITS-REST.

use ferrohealth_ui::html::Html;
use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::search::{Hit, HitGroup, Mark, SearchAnswer, find_results};
use ferrohealth_ui::view::surface::{Header, page_heading};
use ferrohealth_ui::view::template::list::{ListCard, list_card};

use crate::locale::overview::OverviewMsg;
use crate::locale::{Msg, current, tr, tr_with};
use crate::markup::{h, t};
use crate::pages::common::{info_note, shortened};
use crate::routes::Drawn;

/// One kind of hit, as the answer groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// An EHR.
    Ehr,
    /// A composition.
    Composition,
    /// A party.
    Party,
    /// A template.
    Template,
    /// A stored query.
    StoredQuery,
}

impl Kind {
    /// The kind's heading in the answer.
    fn words(self) -> String {
        match self {
            Self::Ehr => tr(Msg::NavEhrs),
            Self::Composition => tr(OverviewMsg::FindCompositions),
            Self::Party => tr(Msg::NavParties),
            Self::Template => tr(Msg::NavTemplates),
            Self::StoredQuery => tr(Msg::NavStoredQueries),
        }
    }

    /// The glyph of a hit of this kind.
    const fn glyph(self) -> Glyph {
        match self {
            Self::Ehr => Glyph::DATABASE,
            Self::Composition => Glyph::FILE_TEXT,
            Self::Party => Glyph::USERS,
            Self::Template => Glyph::FILE_CODE,
            Self::StoredQuery => crate::shell::STORED_QUERIES_GLYPH,
        }
    }
}

/// One thing the CDR holds under the words typed.
#[derive(Debug, Clone)]
struct Found1 {
    /// Its kind.
    kind: Kind,
    /// Its name.
    name: String,
    /// One line on it.
    line: String,
    /// Its page.
    href: String,
}

/// What a search found: the words typed and every hit.
#[derive(Debug, Clone)]
pub struct Found {
    /// The words typed.
    typed: String,
    /// The EHR the words would name, read as an EHR id.
    ehr_href: String,
    /// Every hit, in kind order.
    hits: Vec<Found1>,
    /// Whether the words read as an id.
    id_shaped: bool,
    /// The kinds the CDR did not answer for, so the search is not whole.
    failed: Vec<Kind>,
}

impl Found {
    /// What the search found, every hit grouped by kind: the panel under
    /// the search box shows five of a kind, and the find page every one.
    #[must_use]
    pub fn answer(self) -> SearchAnswer {
        let unfinished = (!self.failed.is_empty()).then(|| failed_line(&self.failed));
        if self.hits.is_empty() {
            let none = if self.failed.is_empty() {
                (
                    tr(OverviewMsg::FindNothingTitle),
                    tr(OverviewMsg::FindNothingLine),
                )
            } else {
                (
                    tr(OverviewMsg::FindUnfinishedTitle),
                    failed_line(&self.failed),
                )
            };
            return SearchAnswer {
                query: self.typed,
                head: None,
                groups: Vec::new(),
                none: Some(none),
                unfinished: None,
                more: None,
            };
        }
        let total = u64::try_from(self.hits.len()).unwrap_or_default();
        let mut groups: Vec<HitGroup> = Vec::new();
        for kind in [
            Kind::Ehr,
            Kind::Composition,
            Kind::Party,
            Kind::Template,
            Kind::StoredQuery,
        ] {
            let hits: Vec<Hit> = self
                .hits
                .iter()
                .filter(|hit| hit.kind == kind)
                .map(|hit| Hit {
                    href: hit.href.clone(),
                    name: hit.name.clone(),
                    line: hit.line.clone(),
                    mark: Some(Mark::Glyph(kind.glyph())),
                    tag: None,
                })
                .collect();
            if !hits.is_empty() {
                groups.push(HitGroup {
                    kind: kind.words(),
                    hits,
                });
            }
        }
        SearchAnswer {
            head: Some(tr_with(
                OverviewMsg::FindMatches,
                &[
                    ("count", total.into()),
                    ("typed", self.typed.clone().into()),
                ],
            )),
            query: self.typed,
            groups,
            none: None,
            unfinished,
            more: None,
        }
    }
}

/// Answers the panel under the search box: what the CDR holds under the
/// words `q`, which the answer route writes as JSON. The words travel in
/// the post's body, never a URL.
///
/// # Errors
/// Returns [`crate::error::ViewerError::Unauthenticated`] with no live
/// session.
pub async fn find_answer(
    // The words typed.
    q: String,
) -> Result<SearchAnswer, crate::error::ViewerError> {
    crate::session::require_session()?;
    Ok(find(&q).await.answer())
}

/// Reads what the CDR holds under the words `q`, for the find page.
///
/// # Errors
/// Returns [`crate::error::ViewerError::Unauthenticated`] with no live
/// session.
pub async fn load_found(
    // The words typed.
    q: String,
) -> Result<Found, crate::error::ViewerError> {
    crate::session::require_session()?;
    Ok(find(&q).await)
}

/// The sentence naming the kinds the CDR did not answer for.
fn failed_line(failed: &[Kind]) -> String {
    let kinds = failed
        .iter()
        .map(|kind| kind.words())
        .collect::<Vec<_>>()
        .join(", ");
    tr_with(OverviewMsg::FindUnfinishedLine, &[("kinds", kinds.into())])
}

/// The hits of the words `typed`, from every place the CDR answers.
pub async fn find(typed: &str) -> Found {
    let typed = typed.trim().to_owned();
    let id = crate::uid::container_uid_of(&typed);
    let id_shaped = crate::bff::ehrs::is_uuid(&id);
    let mut hits = Vec::new();
    let mut failed = Vec::new();
    if id_shaped {
        match crate::bff::ehr_detail::fetch_ehr_summary(id.clone()).await {
            Ok(summary) => hits.push(Found1 {
                kind: Kind::Ehr,
                name: tr_with(
                    OverviewMsg::FindEhr,
                    &[("id", shortened(&summary.ehr_id).into())],
                ),
                line: tr_with(
                    OverviewMsg::FindEhrLine,
                    &[("created", summary.time_created.clone().into())],
                ),
                href: crate::pages::ehr::ehr_href(&summary.ehr_id),
            }),
            Err(error) if crate::pages::common::is_status(&error, http::StatusCode::NOT_FOUND) => {}
            Err(_) => failed.push(Kind::Ehr),
        }
        match composition_hits(&id).await {
            Ok(found) => hits.extend(found),
            Err(_) => failed.push(Kind::Composition),
        }
        match crate::bff::demographics::resolve_party_kind(id.clone()).await {
            Ok(Some(kind)) => hits.push(Found1 {
                kind: Kind::Party,
                name: shortened(&id),
                line: tr_with(OverviewMsg::FindPartyLine, &[("kind", kind.clone().into())]),
                href: format!("/parties/{kind}/{}", urlencoding::encode(&id)),
            }),
            Ok(None) => {}
            Err(_) => failed.push(Kind::Party),
        }
    }
    let needle = typed.to_lowercase();
    if !needle.is_empty() {
        match crate::bff::templates::list_templates().await {
            Ok(templates) => hits.extend(
                templates
                    .into_iter()
                    .filter(|row| {
                        row.template_id.to_lowercase().contains(&needle)
                            || row.concept.to_lowercase().contains(&needle)
                    })
                    .map(|row| Found1 {
                        kind: Kind::Template,
                        line: tr_with(
                            OverviewMsg::FindTemplateLine,
                            &[("concept", row.concept.into())],
                        ),
                        href: format!("/templates/{}", urlencoding::encode(&row.template_id)),
                        name: row.template_id,
                    }),
            ),
            Err(_) => failed.push(Kind::Template),
        }
        match crate::queries_api::list_stored_queries().await {
            Ok(queries) => hits.extend(
                queries
                    .into_iter()
                    .filter(|row| row.name.to_lowercase().contains(&needle))
                    .map(|row| Found1 {
                        kind: Kind::StoredQuery,
                        line: tr_with(
                            OverviewMsg::FindQueryLine,
                            &[("version", row.version.clone().into())],
                        ),
                        href: crate::pages::queries::stored_query_href(&row.name, &row.version),
                        name: row.name,
                    }),
            ),
            Err(_) => failed.push(Kind::StoredQuery),
        }
    }
    Found {
        ehr_href: crate::pages::ehr::ehr_href(&typed),
        typed,
        hits,
        id_shaped,
        failed,
    }
}

/// The compositions whose versioned object is `uid`, read with one AQL query
/// whose value travels as a binding.
#[expect(
    clippy::disallowed_types,
    reason = "the AQL parameters are a JSON object the query wire takes"
)]
async fn composition_hits(uid: &str) -> Result<Vec<Found1>, crate::error::ViewerError> {
    const AQL: &str = "SELECT e/ehr_id/value, c/uid/value, c/name/value \
                       FROM EHR e CONTAINS COMPOSITION c WHERE c/uid/value LIKE $uid";
    let parameters = serde_json::json!({ "uid": format!("{uid}::*") }).to_string();
    let page = crate::queries_api::run_aql(AQL.to_owned(), parameters, 0).await?;
    Ok(page
        .rows
        .iter()
        .filter_map(|row| {
            let ehr = row.first().map(crate::bff::ehrs::cell_text)?;
            let version = row.get(1).map(crate::bff::ehrs::cell_text)?;
            let name = row
                .get(2)
                .map(crate::bff::ehrs::cell_text)
                .unwrap_or_default();
            Some(Found1 {
                kind: Kind::Composition,
                name,
                line: tr_with(
                    OverviewMsg::FindCompositionLine,
                    &[("ehr", shortened(&ehr).into())],
                ),
                href: crate::pages::composition::composition_href(
                    &ehr,
                    &crate::uid::container_uid_of(&version),
                ),
            })
        })
        .collect())
}

/// The ways an id can be read when the CDR holds nothing under it.
fn ways(found: &Found) -> Html {
    let rows = [
        (
            OverviewMsg::FindWaySubject,
            OverviewMsg::FindWaySubjectLine,
            "/ehrs".to_owned(),
        ),
        (
            OverviewMsg::FindWayParty,
            OverviewMsg::FindWayPartyLine,
            "/parties".to_owned(),
        ),
        (
            OverviewMsg::FindWayEhr,
            OverviewMsg::FindWayEhrLine,
            found.ehr_href.clone(),
        ),
    ];
    let items: Html = rows
        .into_iter()
        .map(|(title, line, href)| {
            h(format!(
                r#"<li><a class="fui-row-link" href="{}">{}</a><div class="fui-small fui-muted">{}</div></li>"#,
                t(href),
                t(tr(title)),
                t(tr(line))
            ))
        })
        .collect();
    h(format!(r#"<ul class="fui-match-list">{items}</ul>"#))
}

/// The find page: what was typed, and where it can go.
pub async fn page() -> Drawn {
    let typed = crate::posted::posted()
        .map(|posted| posted.text("q"))
        .filter(|typed| !typed.is_empty());
    let locale = current();
    let heading = page_heading(
        &locale,
        Header {
            title: tr(OverviewMsg::FindTitle).into(),
            subtitle: Some(tr(OverviewMsg::FindSubtitle).into()),
            crumbs: Vec::new(),
            mono: false,
            badge: None,
            subtitle_view: None,
        },
        ferrohealth_ui::view::action::ActionGroup::none(),
    );
    let Some(typed) = typed else {
        let body = heading.then(info_note(t(tr(OverviewMsg::FindEmpty))));
        return Drawn::page(tr(OverviewMsg::FindTitle), body);
    };
    let found = match load_found(typed).await {
        Ok(found) => found,
        Err(error) => {
            let body = heading.then(crate::pages::common::refused(
                tr(OverviewMsg::FindUnfinishedTitle),
                &error,
                None,
            ));
            return Drawn::page(tr(OverviewMsg::FindTitle), body);
        }
    };
    let read = if found.hits.is_empty() {
        let line = if found.id_shaped {
            tr_with(
                OverviewMsg::FindNoIdHolds,
                &[("id", shortened(&found.typed).into())],
            )
        } else {
            tr_with(
                OverviewMsg::FindNothingFor,
                &[("typed", found.typed.clone().into())],
            )
        };
        let failed = if found.failed.is_empty() {
            Html::empty()
        } else {
            crate::pages::common::warn_note(t(failed_line(&found.failed)))
        };
        failed.then(info_note(t(line)))
    } else {
        find_results(&locale, &found.clone().answer())
    };
    let card = list_card(
        &locale,
        ListCard {
            title: tr(OverviewMsg::FindOtherWays),
            count: Some(ferrohealth_ui::view::template::list::HeadingCount::All(3)),
            order: None,
            tabs: None,
            actions: None,
            body: ways(&found),
            pager: None,
        },
    );
    let body = heading.then(h(format!(
        r#"<div class="fui-list-page">{read}{card}</div>"#
    )));
    Drawn::page(tr(OverviewMsg::FindTitle), body)
}

#[cfg(test)]
mod tests {

    #[test]
    fn an_id_is_read_as_a_uuid_by_its_shape() {
        assert!(crate::bff::ehrs::is_uuid(
            "0199c2a4-7e1b-4c3e-9a51-4f2d8b6e1a01"
        ));
        assert!(!crate::bff::ehrs::is_uuid("vital signs"));
    }
}
