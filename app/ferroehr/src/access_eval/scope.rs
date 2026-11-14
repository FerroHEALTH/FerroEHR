// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which data a directive, a label or a classification covers, as a
//! predicate over one item of an EHR.
//!
//! An `ACCESS_DATA_SCOPE` names data by single versioned objects, templates,
//! archetypes, categories, sensitivities, origins and commit time; within one
//! scope every attribute it sets must hold, and a list of scopes covers what
//! any of them covers, an empty list the whole EHR, future data included
//! (draft BMM `openehr_access_control_0.1.0`, `ACCESS_DIRECTIVE.data`). A
//! category is read through the deployment's category map, template first and
//! root archetype for an object whose template the map does not name
//! ([`crate::system_log::categories`]); a sensitivity through the EHR's
//! `ACCESS_LABEL`s and the controller's `ACCESS_CLASSIFICATION`s.
//!
//! One [`Pred`] is evaluated two ways: here against an [`Item`] in memory,
//! and in SQL against a query's row (`aql::sql::access`), over the
//! same lists, so a named object and a query row are judged alike. A bound or
//! a reference that cannot be read covers everything a deny names and
//! nothing a permit names ([`Polarity`]), so an unreadable fact is never
//! read permissively.
//!
//! Read for a settings class ([`crate::access_eval::class`]), a value that
//! is particular to each EHR is [`Held::Fact`]: a predicate names the fact by
//! its position and the query reads it from the EHR's projected facts, so one
//! predicate serves every EHR of the class. No openEHR spec governs this: our
//! own design (#5513, #5588).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use openehr_rm::draft::access_control::access_control::access_classification::AccessClassification;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_label::AccessLabel;
use openehr_rm::draft::access_control::terminology::{ForeignCode, ForeignKind};
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use uuid::Uuid;

use crate::access_eval::class::{DataLayout, Layout};
use crate::policy_register::CategoriesInForce;
use crate::service::ehr::access_projection::{Bound, Validity, validity};

/// How a fact that cannot be read is read: as covering for a deny, as not
/// covering for a permit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Polarity {
    /// The fact narrows what a deny refuses: unreadable, it covers.
    Deny,
    /// The fact narrows what a permit admits: unreadable, it does not.
    Permit,
}

impl Polarity {
    /// The answer for a fact that cannot be read.
    #[must_use]
    pub const fn unreadable(self) -> bool {
        matches!(self, Self::Deny)
    }
}

/// One versioned object of an EHR, as the evaluator reads it: its id, the
/// template and root archetype of its content, the systems it came from and
/// when it was committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Object<'a> {
    /// The versioned object's id.
    pub vo_id: Uuid,
    /// The template the content was committed against.
    pub template: Option<&'a str>,
    /// The `archetype_node_id` of the content's root.
    pub archetype: Option<&'a str>,
    /// The originating systems of the content (`version.origins`); `None`
    /// when nothing stamped them.
    pub origins: Option<&'a [String]>,
    /// When the content was committed.
    pub recorded: Option<jiff::Timestamp>,
}

/// What an access reaches: the EHR as a whole, such as `GET /ehr/{ehr_id}`,
/// which no data scope covers, or one of its objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item<'a> {
    /// The EHR itself.
    Ehr,
    /// One versioned object of the EHR.
    Object(Object<'a>),
}

/// The templates and root archetypes the category map puts in one category.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CategoryMatch {
    /// The templates of the category, case-folded.
    pub templates: Vec<String>,
    /// The archetypes of the category, case-folded, read for an object whose
    /// template the map names nowhere.
    pub archetypes: Vec<String>,
    /// Every template the map names, case-folded.
    pub mapped: Arc<[String]>,
}

impl CategoryMatch {
    /// Whether `object` is of the category.
    fn holds(&self, object: &Object<'_>) -> bool {
        let template = object.template.map(str::to_lowercase);
        if let Some(template) = &template {
            if self.templates.contains(template) {
                return true;
            }
            if self.mapped.contains(template) {
                return false;
            }
        }
        object
            .archetype
            .is_some_and(|archetype| self.archetypes.contains(&archetype.to_lowercase()))
    }
}

/// A value of one EHR's settings: known here, or the fact at a position of
/// the EHR's projected facts, which a class leaves to each of its EHRs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Held<T> {
    /// The value itself.
    Known(T),
    /// The position of the fact that holds it.
    Fact(usize),
}

impl<T> Held<T> {
    /// The value, `None` for a fact.
    #[must_use]
    pub fn known(&self) -> Option<&T> {
        match self {
            Self::Known(value) => Some(value),
            Self::Fact(_) => None,
        }
    }
}

/// One bound of a `recorded` interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    /// The instant.
    pub at: Held<jiff::Timestamp>,
    /// Whether the instant itself is inside.
    pub included: bool,
}

impl From<Bound> for Edge {
    fn from(bound: Bound) -> Self {
        Self {
            at: Held::Known(bound.at),
            included: bound.included,
        }
    }
}

/// The instants a `recorded` interval holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Recorded {
    /// The lower bound; `None` when unbounded.
    pub from: Option<Edge>,
    /// The upper bound; `None` when unbounded.
    pub until: Option<Edge>,
}

impl Recorded {
    /// Whether `at` is inside; `None` when a bound is a fact.
    #[must_use]
    pub fn holds(&self, at: jiff::Timestamp) -> Option<bool> {
        let bound = |edge: Option<Edge>| match edge {
            None => Some(None),
            Some(edge) => edge.at.known().map(|known| {
                Some(Bound {
                    at: *known,
                    included: edge.included,
                })
            }),
        };
        Some(within(bound(self.from)?, bound(self.until)?, at))
    }
}

/// Whether `at` is between `from` and `until`, an absent bound unbounded.
#[must_use]
pub fn within(from: Option<Bound>, until: Option<Bound>, at: jiff::Timestamp) -> bool {
    let above = from.is_none_or(|b| if b.included { b.at <= at } else { b.at < at });
    let below = until.is_none_or(|b| if b.included { at <= b.at } else { at < b.at });
    above && below
}

/// The row attributes one `ACCESS_DATA_SCOPE` sets; an attribute it leaves
/// unset holds for every object.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct DataMatch {
    /// `objects`, as versioned-object ids.
    pub objects: Option<Held<Vec<Uuid>>>,
    /// `templates`, case-folded.
    pub templates: Option<Vec<String>>,
    /// `archetypes`, case-folded.
    pub archetypes: Option<Vec<String>>,
    /// `categories`, each through the category map.
    pub categories: Option<Vec<CategoryMatch>>,
    /// `origins`.
    pub origins: Option<Held<Vec<String>>>,
    /// `recorded`.
    pub recorded: Option<Recorded>,
}

impl DataMatch {
    /// Whether the scope sets no row attribute.
    fn is_empty(&self) -> bool {
        self == &Self::default()
    }

    /// Whether `object` is covered; a fact, which only a query reads, is
    /// read by `polarity`.
    fn holds(&self, object: &Object<'_>, polarity: Polarity) -> bool {
        let template = object.template.map(str::to_lowercase);
        let archetype = object.archetype.map(str::to_lowercase);
        self.objects.as_ref().is_none_or(|ids| {
            ids.known()
                .map_or(polarity.unreadable(), |ids| ids.contains(&object.vo_id))
        }) && self
            .templates
            .as_ref()
            .is_none_or(|t| template.as_ref().is_some_and(|x| t.contains(x)))
            && self
                .archetypes
                .as_ref()
                .is_none_or(|a| archetype.as_ref().is_some_and(|x| a.contains(x)))
            && self
                .categories
                .as_ref()
                .is_none_or(|c| c.iter().any(|category| category.holds(object)))
            && self
                .origins
                .as_ref()
                .is_none_or(|o| match (object.origins, o.known()) {
                    (Some(origins), Some(o)) => origins.iter().any(|x| o.contains(x)),
                    _ => polarity.unreadable(),
                })
            && self.recorded.as_ref().is_none_or(|r| {
                object
                    .recorded
                    .and_then(|at| r.holds(at))
                    .unwrap_or(polarity.unreadable())
            })
    }
}

/// A predicate over the items of one EHR.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Pred {
    /// Every item.
    True,
    /// No item.
    False,
    /// The items the inner predicate leaves out.
    Not(Box<Pred>),
    /// The items every inner predicate holds for.
    All(Vec<Pred>),
    /// The items any inner predicate holds for.
    Any(Vec<Pred>),
    /// The objects a data scope covers, with the polarity an unreadable
    /// commit time is read with; never the EHR itself.
    Data(Arc<DataMatch>, Polarity),
    /// A fact of the EHR as a whole, which only a query of a settings class
    /// reads; it holds for every item of the EHR or for none.
    Ehr(EhrFact),
}

/// A fact of one EHR, named by its position in the EHR's projected facts,
/// that a class verdict leaves to the query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EhrFact {
    /// The request's moment is inside the bounds the facts hold, each with
    /// whether its instant is inside.
    Within {
        /// The lower bound's fact.
        from: Option<(usize, bool)>,
        /// The upper bound's fact.
        until: Option<(usize, bool)>,
    },
    /// The party the fact names is the caller or a party it acts for.
    Party(usize),
    /// The party the fact names is the party the caller is bound to.
    Keeper(usize),
}

impl Pred {
    /// The conjunction of `preds`, folded.
    #[must_use]
    pub fn all(preds: Vec<Self>) -> Self {
        let mut kept = Vec::new();
        for pred in preds {
            match pred {
                Self::True => {}
                Self::False => return Self::False,
                Self::All(inner) => kept.extend(inner),
                other => kept.push(other),
            }
        }
        match kept.len() {
            0 => Self::True,
            1 => kept.pop().unwrap_or(Self::True),
            _ => Self::All(kept),
        }
    }

    /// The disjunction of `preds`, folded.
    #[must_use]
    pub fn any(preds: Vec<Self>) -> Self {
        let mut kept = Vec::new();
        for pred in preds {
            match pred {
                Self::False => {}
                Self::True => return Self::True,
                Self::Any(inner) => kept.extend(inner),
                other => kept.push(other),
            }
        }
        match kept.len() {
            0 => Self::False,
            1 => kept.pop().unwrap_or(Self::False),
            _ => Self::Any(kept),
        }
    }

    /// The negation of `pred`, folded.
    #[must_use]
    pub fn negate(pred: Self) -> Self {
        match pred {
            Self::True => Self::False,
            Self::False => Self::True,
            Self::Not(inner) => *inner,
            other => Self::Not(Box::new(other)),
        }
    }

    /// What the predicate holds for some item, as a predicate over the
    /// facts of the EHR alone: an item scope may hold.
    #[must_use]
    pub fn maybe(&self) -> Self {
        match self {
            Self::True | Self::Data(..) => Self::True,
            Self::False => Self::False,
            Self::Ehr(fact) => Self::Ehr(*fact),
            Self::Not(inner) => Self::negate(inner.always()),
            Self::All(preds) => Self::all(preds.iter().map(Self::maybe).collect()),
            Self::Any(preds) => Self::any(preds.iter().map(Self::maybe).collect()),
        }
    }

    /// What the predicate holds for every item, as a predicate over the
    /// facts of the EHR alone: an item scope may not hold.
    #[must_use]
    pub fn always(&self) -> Self {
        match self {
            Self::True => Self::True,
            Self::False | Self::Data(..) => Self::False,
            Self::Ehr(fact) => Self::Ehr(*fact),
            Self::Not(inner) => Self::negate(inner.maybe()),
            Self::All(preds) => Self::all(preds.iter().map(Self::always).collect()),
            Self::Any(preds) => Self::any(preds.iter().map(Self::always).collect()),
        }
    }

    /// The constant `value`.
    #[must_use]
    pub const fn of(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    /// The value the predicate has for every item, `None` when it depends
    /// on the item.
    #[must_use]
    pub fn constant(&self) -> Option<bool> {
        match self {
            Self::True => Some(true),
            Self::False => Some(false),
            _ => None,
        }
    }

    /// Whether the predicate holds for `item`.
    #[must_use]
    pub fn holds(&self, item: &Item<'_>) -> bool {
        match self {
            Self::True => true,
            // NOTE: our own design (#5588), a direct read resolves every fact, so only a
            // query reads one; read in memory it covers nothing.
            Self::False | Self::Ehr(_) => false,
            Self::Not(inner) => !inner.holds(item),
            Self::All(preds) => preds.iter().all(|pred| pred.holds(item)),
            Self::Any(preds) => preds.iter().any(|pred| pred.holds(item)),
            Self::Data(data, polarity) => match item {
                Item::Ehr => false,
                Item::Object(object) => data.holds(object, *polarity),
            },
        }
    }
}

/// The deployment's category map in force ([`crate::system_log::categories::Classifier::rows`]).
#[derive(Debug, Clone, Default)]
pub struct CategoryIndex {
    source: Option<Arc<CategoriesInForce>>,
}

impl CategoryIndex {
    /// The index of the category map `source` holds.
    #[must_use]
    pub fn new(source: Arc<CategoriesInForce>) -> Self {
        Self {
            source: Some(source),
        }
    }

    /// The templates and archetypes of the category `code` names, as
    /// `<terminology>|<code>` or by its code alone; none when the map holds
    /// no such category.
    fn category(&self, code: &ForeignCode) -> CategoryMatch {
        let Some(classifier) = self.source.as_ref().map(|source| &source.classifier) else {
            return CategoryMatch {
                templates: Vec::new(),
                archetypes: Vec::new(),
                mapped: Arc::from(Vec::new()),
            };
        };
        let token = classifier
            .category_token(&format!("{}|{}", code.terminology, code.code))
            .or_else(|| classifier.category_token(&code.code));
        let (mut templates, mut archetypes, mut mapped) = (Vec::new(), Vec::new(), BTreeSet::new());
        for (kind, key, category) in classifier.rows() {
            let template = *kind == "template";
            if template {
                mapped.insert(key.clone());
            }
            if token.as_ref() == Some(category) {
                if template {
                    templates.push(key.clone());
                } else {
                    archetypes.push(key.clone());
                }
            }
        }
        CategoryMatch {
            templates,
            archetypes,
            mapped: mapped.into_iter().collect::<Vec<_>>().into(),
        }
    }
}

/// The data facts one EHR's directives are read against: the category map,
/// the EHR's labels and the controller's classifications, and, for a
/// settings class, where each fact of its EHRs sits.
#[derive(Debug, Clone, Default)]
pub struct Scopes {
    categories: CategoryIndex,
    /// The objects labelled with each sensitivity.
    labels: BTreeMap<ForeignCode, Held<Vec<Uuid>>>,
    /// Each classification's data and the sensitivities it gives them.
    classifications: Vec<(Pred, Vec<ForeignCode>)>,
    /// The facts of a settings class; `None` for one EHR's own settings.
    layout: Option<Arc<Layout>>,
}

impl Scopes {
    /// The scopes of an EHR labelled `labels`, under the controller's
    /// `classifications`, read through `categories`.
    #[must_use]
    pub fn new(
        categories: CategoryIndex,
        labels: &[AccessLabel],
        classifications: &[AccessClassification],
    ) -> Self {
        let mut by_code: BTreeMap<ForeignCode, Vec<Uuid>> = BTreeMap::new();
        for label in labels {
            // NOTE: our own design, a label on a reference that names no versioned object
            // labels nothing, since nothing it names can be read.
            let Some(vo_id) = crate::service::ehr::access_marks::object_id(&label.target) else {
                continue;
            };
            for code in sensitivities(&label.sensitivities) {
                by_code.entry(code).or_default().push(vo_id);
            }
        }
        let labels = by_code
            .into_iter()
            .map(|(code, objects)| (code, Held::Known(objects)))
            .collect();
        Self::with(categories, labels, classifications, None)
    }

    /// The scopes of every EHR of a settings class laid out as `layout`,
    /// whose labelled objects are facts, under the controller's
    /// `classifications`, read through `categories`.
    #[must_use]
    pub fn of_class(
        categories: CategoryIndex,
        layout: Arc<Layout>,
        classifications: &[AccessClassification],
    ) -> Self {
        let labels = layout
            .labels
            .iter()
            .map(|(code, fact)| (code.clone(), Held::Fact(*fact)))
            .collect();
        Self::with(categories, labels, classifications, Some(layout))
    }

    /// The scopes of `labels` under `classifications`, laid out as `layout`.
    fn with(
        categories: CategoryIndex,
        labels: BTreeMap<ForeignCode, Held<Vec<Uuid>>>,
        classifications: &[AccessClassification],
        layout: Option<Arc<Layout>>,
    ) -> Self {
        let mut scopes = Self {
            categories,
            labels,
            classifications: Vec::new(),
            layout,
        };
        scopes.classifications = classifications
            .iter()
            .map(|classification| {
                (
                    scopes.scope(&classification.data, Polarity::Deny, false, None),
                    sensitivities(&classification.sensitivities),
                )
            })
            .collect();
        scopes
    }

    /// Every sensitivity a label or a classification gives.
    #[must_use]
    pub fn sensitivity_codes(&self) -> BTreeSet<ForeignCode> {
        self.labels
            .keys()
            .cloned()
            .chain(
                self.classifications
                    .iter()
                    .flat_map(|(_, codes)| codes.iter().cloned()),
            )
            .collect()
    }

    /// The items that carry `code`: labelled with it, or of a class the
    /// controller gives it.
    #[must_use]
    pub fn sensitive(&self, code: &ForeignCode) -> Pred {
        let mut preds = Vec::new();
        if let Some(objects) = self.labels.get(code) {
            preds.push(labelled(objects, Polarity::Deny));
        }
        for (pred, codes) in &self.classifications {
            if codes.contains(code) {
                preds.push(pred.clone());
            }
        }
        Pred::any(preds)
    }

    /// The layout of the settings class these scopes read; `None` for one
    /// EHR's own settings.
    #[must_use]
    pub fn layout(&self) -> Option<&Layout> {
        self.layout.as_deref()
    }

    /// The items `scopes` cover, the whole EHR when empty; `layout` places
    /// their facts when these scopes read a settings class.
    #[must_use]
    pub fn data(
        &self,
        scopes: &[AccessDataScope],
        polarity: Polarity,
        layout: Option<&[DataLayout]>,
    ) -> Pred {
        if scopes.is_empty() {
            return Pred::True;
        }
        Pred::any(
            scopes
                .iter()
                .enumerate()
                .map(|(at, scope)| {
                    let placed = layout.and_then(|layout| layout.get(at));
                    self.scope(scope, polarity, true, placed)
                })
                .collect(),
        )
    }

    /// The items one scope covers; `with_sensitivity` reads its
    /// sensitivities through the labels and classifications, which a
    /// classification's own scope does not, so classifications never refer
    /// to each other.
    fn scope(
        &self,
        scope: &AccessDataScope,
        polarity: Polarity,
        with_sensitivity: bool,
        placed: Option<&DataLayout>,
    ) -> Pred {
        let objects = scope.objects.as_deref().map(|objects| {
            objects
                .iter()
                .filter_map(crate::service::ehr::access_marks::object_id)
                .collect::<Vec<_>>()
        });
        // NOTE: our own design, an object reference that names no object covers every
        // object for a deny and none for a permit.
        let unplaced = scope
            .objects
            .as_deref()
            .is_some_and(|all| objects.as_ref().is_some_and(|read| read.len() < all.len()));
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
            recorded,
        };
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
        Pred::all(vec![rows, sensitivity])
    }
}

/// `known`, or the fact at `at` when the class leaves it to each EHR.
fn held<T>(known: T, at: Option<usize>) -> Held<T> {
    at.map_or(Held::Known(known), Held::Fact)
}

/// The objects `objects` labels, read with `polarity`.
fn labelled(objects: &Held<Vec<Uuid>>, polarity: Polarity) -> Pred {
    Pred::Data(
        Arc::new(DataMatch {
            objects: Some(objects.clone()),
            ..DataMatch::default()
        }),
        polarity,
    )
}

/// The sensitivity codes of `coded`, each of the jurisdiction's terminology;
/// the write check refuses any other, so one that does not read is left out.
fn sensitivities(coded: &[DvCodedText]) -> Vec<ForeignCode> {
    coded
        .iter()
        .filter_map(|coded| ForeignCode::read(ForeignKind::Sensitivity, coded).ok())
        .collect()
}
