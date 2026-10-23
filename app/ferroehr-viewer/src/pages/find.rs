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
//! EHRs list asks for. The answer opens under the search box, grouped by
//! kind, and the find page lists the other ways to read the id. No openEHR
//! spec governs the viewer: our own design; the reads are ITS-REST.

use ferrohealth_ui::icon::Glyph;
use ferrohealth_ui::view::search::{Hit, HitGroup, Mark, SearchAnswer};
use ferrohealth_ui::view::surface::{Header, page_heading};
use ferrohealth_ui::view::template::list::{ListCard, list_card};
use leptos::prelude::*;
use leptos_meta::Title;

use crate::locale::{Msg, tr, tr_with};
use crate::pages::common::{awaited, info_note, shortened};

/// The most hits one kind shows under the search box.
const HITS_PER_KIND: usize = 5;

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
        tr(match self {
            Self::Ehr => Msg::NavEhrs,
            Self::Composition => Msg::FindCompositions,
            Self::Party => Msg::NavParties,
            Self::Template => Msg::NavTemplates,
            Self::StoredQuery => Msg::NavStoredQueries,
        })
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
    /// Every hit, in kind order.
    hits: Vec<Found1>,
    /// Whether the words read as an id.
    id_shaped: bool,
    /// The kinds the CDR did not answer for, so the search is not whole.
    failed: Vec<Kind>,
}

impl Found {
    /// The answer the top bar's search opens under it.
    #[must_use]
    pub fn answer(self) -> SearchAnswer {
        if self.hits.is_empty() {
            let none = if self.failed.is_empty() {
                (tr(Msg::FindNothingTitle), tr(Msg::FindNothingLine))
            } else {
                (tr(Msg::FindUnfinishedTitle), failed_line(&self.failed))
            };
            return SearchAnswer {
                head: None,
                groups: Vec::new(),
                none: Some(none),
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
                .take(HITS_PER_KIND)
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
                Msg::FindMatches,
                &[("count", total.into()), ("typed", self.typed.into())],
            )),
            groups,
            none: None,
            more: None,
        }
    }
}

/// The sentence naming the kinds the CDR did not answer for.
fn failed_line(failed: &[Kind]) -> String {
    let kinds = failed
        .iter()
        .map(|kind| kind.words())
        .collect::<Vec<_>>()
        .join(", ");
    tr_with(Msg::FindUnfinishedLine, &[("kinds", kinds.into())])
}

/// Whether `error` means the thing looked for is not there (`404`), which a
/// search reads as no hit; any other error is the CDR not answering.
fn absent(error: &crate::error::ViewerError) -> bool {
    crate::pages::common::is_status(error, http::StatusCode::NOT_FOUND)
}

/// Whether `text` is shaped as an RFC 9562 UUID.
fn uuid_shaped(text: &str) -> bool {
    crate::bff::ehrs::is_uuid(text)
}

/// The hits of the words `typed`, from every place the CDR answers.
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
                name: tr_with(Msg::FindEhr, &[("id", shortened(&summary.ehr_id).into())]),
                line: tr_with(
                    Msg::FindEhrLine,
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
                line: tr_with(Msg::FindPartyLine, &[("kind", kind.clone().into())]),
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
                        line: tr_with(Msg::FindTemplateLine, &[("concept", row.concept.into())]),
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
                            Msg::FindQueryLine,
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
                line: tr_with(Msg::FindCompositionLine, &[("ehr", shortened(&ehr).into())]),
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
            Msg::FindWaySubject,
            Msg::FindWaySubjectLine,
            "/ehrs".to_owned(),
        ),
        (
            Msg::FindWayParty,
            Msg::FindWayPartyLine,
            "/parties".to_owned(),
        ),
        (
            Msg::FindWayEhr,
            Msg::FindWayEhrLine,
            crate::bff::ehrs::ehr_detail_href(&found.typed),
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
    let body = awaited(async move {
        let heading = page_heading(
            Header {
                title: tr(Msg::FindTitle).into(),
                subtitle: Some(tr(Msg::FindSubtitle).into()),
                crumbs: Vec::new(),
                mono: false,
                badge: None,
                subtitle_view: None,
            },
            ferrohealth_ui::view::action::ActionGroup::none(),
        )
        .into_any();
        let Some(typed) = typed else {
            return view! {
                {heading}
                {info_note(tr(Msg::FindEmpty))}
            }
            .into_any();
        };
        let found = find(&typed).await;
        let unfinished = (!found.failed.is_empty())
            .then(|| crate::pages::common::warn_note(failed_line(&found.failed)));
        let read = if found.hits.is_empty() {
            let line = if found.id_shaped {
                tr_with(
                    Msg::FindNoIdHolds,
                    &[("id", shortened(&found.typed).into())],
                )
            } else {
                tr_with(
                    Msg::FindNothingFor,
                    &[("typed", found.typed.clone().into())],
                )
            };
            info_note(line)
        } else {
            list_card(ListCard {
                title: tr_with(
                    Msg::FindFoundSome,
                    &[("count", crate::pages::common::count(found.hits.len()))],
                ),
                count: None,
                order: None,
                tabs: None,
                actions: None,
                body: ferrohealth_ui::view::search::search_answer(
                    "find-page",
                    found.clone().answer(),
                ),
                pager: None,
            })
        };
        let card = list_card(ListCard {
            title: tr(Msg::FindOtherWays),
            count: Some(ferrohealth_ui::view::template::list::HeadingCount::All(3)),
            order: None,
            tabs: None,
            actions: None,
            body: ways(&found),
            pager: None,
        });
        view! {
            {heading}
            <div class="fui-list-page">{unfinished} {read} {card}</div>
        }
        .into_any()
    });
    view! {
        <Title text=tr(Msg::FindTitle) />
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
