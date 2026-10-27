// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The top bar's search and the find page: what the CDR holds under the words
//! typed, and where an id can be opened.
//!
//! The search posts its words, since an id can name a patient's record, so
//! they never reach a URL (FerroUI `leptos-ui.md` §5). An id shaped as a UUID
//! is read as an EHR id and as a composition's versioned object, and is tried
//! as a party; any words are matched against the template ids and concepts
//! and the stored queries' names. A subject id needs its namespace, which the
//! EHRs list asks for. As you type, the panel under the search box asks the
//! server function [`find_answer`] and shows the hits grouped by kind; the
//! find page lists every hit in the page and the other ways to read the id.
//! No openEHR spec governs the viewer: our own design; the reads are
//! ITS-REST.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::search::{Hit, HitGroup, Mark, SearchAnswer, find_results};
use ferrohealth_ui::view::surface::{Header, page_heading};
use ferrohealth_ui::view::template::list::{ListCard, list_card};
use leptos::prelude::*;
use leptos::server_fn::codec::Json;
use leptos_meta::Title;
use serde::{Deserialize, Serialize};

use crate::locale::overview::OverviewMsg;
use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{awaited, info_note, shortened};
use crate::session_client::SessionAwareClient;

/// One kind of hit, as the answer groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// What a search found: the words typed and every hit, read on the server
/// and drawn on both halves.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
/// words `q`, as JSON. The words travel in the post's body, never a URL.
///
/// # Errors
/// Returns [`crate::error::ViewerError::Unauthenticated`] with no live
/// session.
#[server(endpoint = "find_answer", output = Json, client = SessionAwareClient)]
pub async fn find_answer(
    /// The words typed.
    q: String,
) -> Result<SearchAnswer, crate::error::ViewerError> {
    crate::session::require_session().await?;
    Ok(find(&q).await.answer())
}

/// Reads what the CDR holds under the words `q`, for the find page.
///
/// # Errors
/// Returns [`crate::error::ViewerError::Unauthenticated`] with no live
/// session.
#[server(client = SessionAwareClient)]
pub async fn load_found(
    /// The words typed.
    q: String,
) -> Result<Found, crate::error::ViewerError> {
    crate::session::require_session().await?;
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

/// Whether `error` means the thing looked for is not there (`404`), which a
/// search reads as no hit; any other error is the CDR not answering.
#[cfg(not(target_arch = "wasm32"))]
fn absent(error: &crate::error::ViewerError) -> bool {
    crate::pages::common::is_status(error, http::StatusCode::NOT_FOUND)
}

/// Whether `text` is shaped as an RFC 9562 UUID.
#[cfg(not(target_arch = "wasm32"))]
fn uuid_shaped(text: &str) -> bool {
    crate::bff::ehrs::is_uuid(text)
}

/// The hits of the words `typed`, from every place the CDR answers.
#[cfg(not(target_arch = "wasm32"))]
pub async fn find(typed: &str) -> Found {
    let typed = typed.trim().to_owned();
    let id = crate::uid::container_uid_of(&typed);
    let id_shaped = uuid_shaped(&id);
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
                href: crate::bff::ehrs::ehr_detail_href(&summary.ehr_id),
            }),
            Err(error) if absent(&error) => {}
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
        ehr_href: crate::bff::ehrs::ehr_detail_href(&typed),
        typed,
        hits,
        id_shaped,
        failed,
    }
}

/// The compositions whose versioned object is `uid`, read with one AQL query
/// whose value travels as a binding.
#[cfg(not(target_arch = "wasm32"))]
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
fn ways(found: &Found) -> AnyView {
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
    let items = rows
        .into_iter()
        .map(|(title, line, href)| {
            view! {
                <li>
                    <a class="fui-row-link" href=href>
                        {tr(title)}
                    </a>
                    <div class="fui-small fui-muted">{tr(line)}</div>
                </li>
            }
        })
        .collect_view();
    view! { <ul class="fui-match-list">{items}</ul> }.into_any()
}

/// The find page: what was typed, and where it can go.
#[expect(
    clippy::must_use_candidate,
    reason = "#[component] rewrites the fn; the router always consumes the value"
)]
#[component]
pub fn FindPage() -> impl IntoView {
    let typed = crate::posted::posted()
        .map(|posted| posted.text("q"))
        .filter(|typed| !typed.is_empty());
    let model = OnceResource::new(async move {
        match typed {
            Some(typed) => load_found(typed).await.map(Some),
            None => Ok(None),
        }
    });
    let body = awaited(async move {
        let heading = page_heading(
            Header {
                title: tr(OverviewMsg::FindTitle).into(),
                subtitle: Some(tr(OverviewMsg::FindSubtitle).into()),
                crumbs: Vec::new(),
                mono: false,
                badge: None,
                subtitle_view: None,
            },
            ferrohealth_ui::view::action::ActionGroup::none(),
        )
        .into_any();
        let found = match model.await {
            Ok(Some(found)) => found,
            Ok(None) => {
                return view! {
                    {heading}
                    {info_note(tr(OverviewMsg::FindEmpty))}
                }
                .into_any();
            }
            Err(error) => {
                return view! {
                    {heading}
                    {crate::pages::common::refused(
                        tr(OverviewMsg::FindUnfinishedTitle),
                        &error,
                        None,
                    )}
                }
                .into_any();
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
            view! {
                {(!found.failed.is_empty())
                    .then(|| crate::pages::common::warn_note(failed_line(&found.failed)))}
                {info_note(line)}
            }
            .into_any()
        } else {
            find_results(found.clone().answer())
        };
        let card = list_card(ListCard {
            title: tr(OverviewMsg::FindOtherWays),
            count: Some(ferrohealth_ui::view::template::list::HeadingCount::All(3)),
            order: None,
            tabs: None,
            actions: None,
            body: ways(&found),
            pager: None,
        });
        view! {
            {heading}
            <div class="fui-list-page">{read} {card}</div>
        }
        .into_any()
    });
    view! {
        <Title text=tr(OverviewMsg::FindTitle) />
        {body}
    }
}

#[cfg(test)]
mod tests {
    use super::uuid_shaped;

    #[test]
    fn an_id_is_read_as_a_uuid_by_its_shape() {
        assert!(uuid_shaped("0199c2a4-7e1b-4c3e-9a51-4f2d8b6e1a01"));
        assert!(!uuid_shaped("vital signs"));
    }
}
