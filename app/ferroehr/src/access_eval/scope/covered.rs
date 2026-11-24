// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The items one `ACCESS_DATA_SCOPE` covers, as a [`Pred`] over the items of
//! an EHR.

use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind};

use super::{
    DataMatch, Edge, Held, Polarity, Pred, Recorded, Scopes, episode_key, labelled, sensitivities,
};
use crate::access_eval::class::DataLayout;
use crate::service::ehr::access_projection::{Bound, Validity, validity};

impl Scopes {
    /// The items one scope covers; `with_sensitivity` reads its
    /// sensitivities through the labels and classifications, which a
    /// classification's own scope does not, so classifications never refer
    /// to each other.
    pub(super) fn scope(
        &self,
        scope: &AccessDataScope,
        polarity: Polarity,
        with_sensitivity: bool,
        placed: Option<&DataLayout>,
    ) -> Pred {
        let objects = scope.objects.as_deref().map(|objects| {
            objects
                .iter()
                .filter_map(crate::service::ehr::access_settings::object_id)
                .collect::<Vec<_>>()
        });
        // NOTE: our own design, an object reference that names no object covers every
        // object for a deny and none for a permit.
        let unplaced = scope
            .objects
            .as_deref()
            .is_some_and(|all| objects.as_ref().is_some_and(|read| read.len() < all.len()));
        let episodes = scope
            .episodes
            .as_deref()
            .map(|named| named.iter().filter_map(episode_key).collect::<Vec<_>>());
        // NOTE: our own design, an episode with no issuer names no episode, so it covers
        // every object for a deny and none for a permit, as an unplaced object reference.
        let unnamed = scope
            .episodes
            .as_deref()
            .is_some_and(|all| episodes.as_ref().is_some_and(|read| read.len() < all.len()));
        let fact = |known: Option<Bound>, at: Option<usize>| {
            known.map(|bound| match at {
                Some(at) => Edge {
                    at: Held::Fact(at),
                    included: bound.included,
                },
                None => Edge::from(bound),
            })
        };
        let recorded = match scope.recorded.as_ref().map(validity) {
            None => None,
            Some(Validity::Within { from, until }) => Some(Recorded {
                from: fact(from, placed.and_then(|p| p.from)),
                until: fact(until, placed.and_then(|p| p.until)),
            }),
            Some(Validity::Unreadable) => {
                return if polarity.unreadable() {
                    Pred::True
                } else {
                    Pred::False
                };
            }
        };
        let data = DataMatch {
            objects: objects
                .filter(|_| !(unplaced && polarity.unreadable()))
                .map(|objects| held(objects, placed.and_then(|p| p.objects))),
            templates: scope
                .templates
                .as_deref()
                .map(|t| t.iter().map(|x| x.to_lowercase()).collect()),
            archetypes: scope
                .archetypes
                .as_deref()
                .map(|a| a.iter().map(|x| x.value.to_lowercase()).collect()),
            categories: scope.categories.as_deref().map(|codes| {
                codes
                    .iter()
                    .filter_map(|coded| ForeignCode::read(ForeignKind::Category, coded).ok())
                    .map(|code| self.categories.category(&code))
                    .collect()
            }),
            origins: scope
                .origins
                .as_deref()
                .map(|origins| held(origins.to_vec(), placed.and_then(|p| p.origins))),
            episodes: episodes
                .filter(|_| !(unnamed && polarity.unreadable()))
                .map(|episodes| held(episodes, placed.and_then(|p| p.episodes))),
            recorded,
        };
        let confidentiality = scope
            .confidentialities
            .as_deref()
            .map_or(Pred::True, |levels| {
                self.at_most(levels, polarity, with_sensitivity)
            });
        let sensitivity = match scope.sensitivities.as_deref() {
            Some(codes) if with_sensitivity => Pred::any(
                sensitivities(codes)
                    .iter()
                    .map(|code| self.sensitive(code))
                    .collect(),
            ),
            Some(codes) => Pred::any(
                sensitivities(codes)
                    .iter()
                    .filter_map(|code| self.labels.get(code))
                    .map(|objects| labelled(objects, polarity))
                    .collect(),
            ),
            None => Pred::True,
        };
        let rows = if data.is_empty() {
            Pred::True
        } else {
            Pred::Data(Arc::new(data), polarity)
        };
        Pred::all(vec![rows, sensitivity, confidentiality])
    }
}

/// `known`, or the fact at `at` when the class leaves it to each EHR.
fn held<T>(known: T, at: Option<usize>) -> Held<T> {
    at.map_or(Held::Known(known), Held::Fact)
}
