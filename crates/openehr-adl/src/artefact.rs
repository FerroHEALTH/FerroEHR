// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Artefact-level views and the parent/supplier repository — the crate-wide
//! access layer over an assembled `openehr_am::v2_4::aom2` [`Archetype`].
//!
//! `ArchetypeView` is the borrowed, artefact-kind-agnostic view of the common
//! fields every `AUTHORED_ARCHETYPE` / `TEMPLATE` / `OPERATIONAL_TEMPLATE` /
//! `TEMPLATE_OVERLAY` carries (`docs/specs/openehr/AM/docs/AOM2/master03-archetype_package.adoc`
//! §Top-level Meta-data + `ADL2/master07.03` §Artefact Categories), so a
//! consumer never matches on the artefact-kind enum
//! itself. [`ArchetypeRepository`] is the minimal in-memory parent/supplier seam
//! that specialisation-aware work resolves against, and [`resolve_flat_parent`]
//! classifies a child's declared parent for it.
//!
//! This module sits BELOW validation, flattening, and OPT generation: all three
//! read through it, and it depends on none of them.

use std::collections::BTreeMap;

use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::archetype_hrid::ArchetypeHrid;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::rm_overlay::rm_overlay::RmOverlay;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;
use openehr_am::v2_4::resource::resource_description::ResourceDescription;
use openehr_base::prelude::{ResourceAnnotations, TerminologyCode, VersionStatus};

use crate::hrid::{hrid_lookup_key, parse_hrid, raw_id_lookup_key, raw_id_version};
use crate::source::ArtefactKind;

/// A minimal in-memory archetype repository — the parent/supplier seam the
/// specialisation-aware checks resolve against.
///
/// An artefact is held under its namespace, its case-folded
/// `publisher-package-class.concept` family and its major version: BASE
/// `artefact_identification` `master07-referencing.adoc` §Referencing makes
/// two major versions of one archetype "different logical archetypes", and a
/// namespace is "never a wildcard". Every release of one key is kept, and a
/// reference resolves within its key by [`ArchetypeRepository::resolve`].
#[derive(Debug, Default, Clone)]
pub struct ArchetypeRepository {
    by_key: BTreeMap<ArtefactKey, Vec<Archetype>>,
}

/// The identity a reference matches before its version is read: namespace,
/// family and major version.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ArtefactKey {
    /// The namespace as the identifier writes it, `None` when it carries none.
    namespace: Option<String>,
    /// The case-folded `publisher-package-class.concept` ([`hrid_lookup_key`]).
    family: String,
    /// The major version, as a number when it is one.
    major: String,
}

impl ArtefactKey {
    /// The key of a stored artefact's identifier.
    fn of(h: &ArchetypeHrid) -> Self {
        Self {
            namespace: h.namespace.clone(),
            family: hrid_lookup_key(h),
            major: major_key(h.major_version()),
        }
    }
}

/// A major version written canonically (`01` → `1`), so a reference and a
/// stored identifier agree on it.
fn major_key(major: &str) -> String {
    major
        .parse::<u64>()
        .map_or_else(|_| major.to_owned(), |n| n.to_string())
}

impl ArchetypeRepository {
    /// Register a parsed archetype under its namespace, family and major
    /// version, replacing an artefact of the same full version.
    pub fn insert(&mut self, archetype: Archetype) {
        let hrid = view(&archetype).archetype_id;
        let held = self.by_key.entry(ArtefactKey::of(hrid)).or_default();
        let same = held
            .iter()
            .position(|a| same_version(view(a).archetype_id, hrid));
        match same.and_then(|i| held.get_mut(i)) {
            Some(slot) => *slot = archetype,
            None => held.push(archetype),
        }
    }

    /// Resolve a raw archetype reference made from an artefact in
    /// `referrer_namespace`.
    ///
    /// A reference without a namespace takes the referrer's, and a reference
    /// resolves only within its namespace, family and major version (BASE
    /// `artefact_identification` `master07-referencing.adoc` §Referencing).
    /// An interface reference (`v1`) or a specific one (`v1.2`) takes the
    /// newest version carrying those version parts, a full reference
    /// (`v1.2.3`) that version, and a release outranks every release
    /// candidate. An alpha or beta version matches only its full identifier.
    #[must_use]
    pub fn resolve(&self, reference: &str, referrer_namespace: Option<&str>) -> Option<&Archetype> {
        let query = Query::of(reference, referrer_namespace)?;
        let held: Vec<&Archetype> = match query.key() {
            Some(key) => self.by_key.get(&key).into_iter().flatten().collect(),
            None => self
                .by_key
                .iter()
                .filter(|(key, _)| query.admits(key))
                .flat_map(|(_, held)| held)
                .collect(),
        };
        query.best(held, |a| view(a).archetype_id)
    }
}

/// The identifier among `held` that `reference`, made from an artefact in
/// `referrer_namespace`, resolves to, by the rule of
/// [`ArchetypeRepository::resolve`].
///
/// This is that rule over bare identifiers, for a caller that holds the
/// identifiers of stored artefacts without their parsed content.
#[must_use]
pub fn resolve_hrid<'a>(
    reference: &str,
    referrer_namespace: Option<&str>,
    held: impl IntoIterator<Item = &'a ArchetypeHrid>,
) -> Option<&'a ArchetypeHrid> {
    let query = Query::of(reference, referrer_namespace)?;
    let held = held
        .into_iter()
        .filter(|h| query.admits(&ArtefactKey::of(h)));
    query.best(held, |h| h)
}

/// One reference, read once: the namespace it resolves in, its family and the
/// version it asks for.
struct Query {
    /// The reference's namespace, else the referrer's.
    namespace: Option<String>,
    /// The case-folded family ([`raw_id_lookup_key`]).
    family: String,
    /// The version asked for.
    wanted: Wanted,
}

impl Query {
    /// The query of `reference`, or `None` when its version is not numeric.
    fn of(reference: &str, referrer_namespace: Option<&str>) -> Option<Self> {
        let (namespace, _) = split_namespace(reference);
        Some(Self {
            namespace: namespace.or(referrer_namespace).map(str::to_owned),
            family: raw_id_lookup_key(reference),
            wanted: Wanted::of(reference)?,
        })
    }

    /// The one key this query reads, `None` for a versionless reference.
    fn key(&self) -> Option<ArtefactKey> {
        self.wanted.major().map(|major| ArtefactKey {
            namespace: self.namespace.clone(),
            family: self.family.clone(),
            major,
        })
    }

    /// Whether artefacts held under `key` are candidates for this query.
    ///
    /// NOTE: no openEHR spec governs a reference without a version
    /// (`master07-referencing.adoc` §Referencing gives every `hrid_ref` a
    /// major version); our own design reads it across every major version.
    fn admits(&self, key: &ArtefactKey) -> bool {
        key.namespace == self.namespace
            && key.family == self.family
            && self.wanted.major().is_none_or(|major| key.major == major)
    }

    /// The candidate of highest precedence among `held` that the wanted
    /// version admits.
    fn best<T>(
        &self,
        held: impl IntoIterator<Item = T>,
        hrid: impl Fn(&T) -> &ArchetypeHrid,
    ) -> Option<T> {
        held.into_iter()
            .filter_map(|t| self.wanted.rank(hrid(&t)).map(|rank| (t, rank)))
            .max_by_key(|(_, rank)| *rank)
            .map(|(t, _)| t)
    }
}

/// A raw identifier split at its `ns::` prefix: the namespace, if any, and the
/// rest.
fn split_namespace(raw: &str) -> (Option<&str>, &str) {
    match raw.rsplit_once("::") {
        Some((namespace, rest)) => (Some(namespace), rest),
        None => (None, raw),
    }
}

/// True when two identifiers name the same full version.
fn same_version(a: &ArchetypeHrid, b: &ArchetypeHrid) -> bool {
    a.release_version == b.release_version
        && a.version_status == b.version_status
        && a.build_count == b.build_count
}

/// What a reference asks of a held version.
enum Wanted {
    /// The leading numeric version parts a release or release candidate must
    /// carry (`v1` → `[1]`, `v1.2` → `[1, 2]`; none for a versionless id).
    Prefix(Vec<u64>),
    /// A pre-release named by its full identifier.
    Exact(ArchetypeHrid),
}

impl Wanted {
    /// The version `reference` asks for, or `None` when its version is not
    /// numeric, so that nothing held matches it.
    fn of(reference: &str) -> Option<Self> {
        if let Ok(h) = parse_hrid(reference)
            && h.version_status != VersionStatus::Released
        {
            return Some(Self::Exact(h));
        }
        let version = raw_id_version(reference);
        if version.is_empty() {
            return Some(Self::Prefix(Vec::new()));
        }
        version
            .split('.')
            .map(|part| part.parse().ok())
            .collect::<Option<Vec<u64>>>()
            .map(Self::Prefix)
    }

    /// The major version this reference is confined to, `None` for a
    /// versionless reference.
    fn major(&self) -> Option<String> {
        match self {
            Self::Prefix(parts) => parts.first().map(u64::to_string),
            Self::Exact(h) => Some(major_key(h.major_version())),
        }
    }

    /// The precedence of `held` when it matches, higher first.
    fn rank(&self, held: &ArchetypeHrid) -> Option<Precedence> {
        let precedence = Precedence::of(held)?;
        let admitted = match self {
            Self::Prefix(parts) => {
                precedence.status != Status::Other && precedence.release.starts_with(parts)
            }
            Self::Exact(h) => same_version(h, held),
        };
        admitted.then_some(precedence)
    }
}

/// The lifecycle of a held version as reference matching ranks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    /// An alpha, beta or build version, matched by its full identifier only.
    Other,
    /// A release candidate.
    Candidate,
    /// A released version.
    Released,
}

/// The order of the held versions one reference matches: released before any
/// release candidate, then the newest release number, then the build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Precedence {
    /// The lifecycle, compared first.
    status: Status,
    /// `major.minor.patch`, an absent part read as `0`.
    release: [u64; 3],
    /// The pre-release build count, `0` when none.
    build: u64,
}

impl Precedence {
    /// The precedence of a held version, or `None` when its version is not
    /// numeric.
    ///
    /// NOTE: `master07-referencing.adoc` §Archetype External References (ADL/AOM 2)
    /// resolves to "the most recent released version … OR the latest release
    /// candidate" without an order; our own design takes the release first.
    fn of(h: &ArchetypeHrid) -> Option<Self> {
        let number = |part: &str| {
            if part.is_empty() {
                Some(0)
            } else {
                part.parse::<u64>().ok()
            }
        };
        let status = match h.version_status {
            VersionStatus::Released => Status::Released,
            VersionStatus::ReleaseCandidate => Status::Candidate,
            _ => Status::Other,
        };
        Some(Self {
            status,
            release: [
                number(h.major_version())?,
                number(h.minor_version())?,
                number(h.patch_version())?,
            ],
            build: number(&h.build_count)?,
        })
    }
}

/// The outcome of resolving a specialised archetype's flat parent — the input
/// the phase-2 specialisation checks validate against.
///
/// A level-0 (non-specialised) parent is its own flat form, so it is returned
/// [`Available`](FlatParent::Available) directly (`ADL2/master09.02`
/// §Differential and Flat Forms: "For a top-level archetype, the flat-form is
/// the same as its differential form"). A parent that is itself specialised
/// needs its deep flat form (produced by [`crate::flatten::flat_form`]), which is
/// owned rather than borrowable, so it is reported
/// [`NeedsFlattener`](FlatParent::NeedsFlattener) and flattened by the caller.
#[derive(Debug, Clone, Copy)]
pub enum FlatParent<'a> {
    /// The archetype is not specialised — the phase-2 specialisation checks do
    /// not apply.
    NotSpecialised,
    /// The flat parent is available (a level-0 parent used as-is).
    Available(&'a Archetype),
    /// The declared parent is registered but is itself specialised, so its deep
    /// flat form is computed (owned) by the caller via
    /// [`crate::flatten::flat_form`] rather than borrowed here.
    NeedsFlattener,
    /// The declared parent could not be resolved in the repository.
    NotFound,
}

/// Resolve `child`'s flat parent from `repo` for the phase-2 specialisation
/// checks.
///
/// Returns [`FlatParent::NotSpecialised`] for a non-specialised archetype,
/// [`FlatParent::NotFound`] when the declared parent is absent from `repo`,
/// [`FlatParent::NeedsFlattener`] when the parent is itself specialised (its
/// deep flat form is not yet computable), and
/// [`FlatParent::Available`] for a level-0 parent (its own flat form).
#[must_use]
pub fn resolve_flat_parent<'a>(child: &Archetype, repo: &'a ArchetypeRepository) -> FlatParent<'a> {
    let Some(parent_id) = view(child).parent_archetype_id else {
        return FlatParent::NotSpecialised;
    };
    let Some(parent) = repo.resolve(parent_id, view(child).archetype_id.namespace.as_deref())
    else {
        return FlatParent::NotFound;
    };
    if view(parent).is_specialised() {
        // A borrowed level-0 parent is its own flat form; a specialised parent's
        // deep flat form is owned (computed by [`crate::flatten::flat_form`]) and
        // cannot be handed back by borrow — the caller flattens it there.
        return FlatParent::NeedsFlattener;
    }
    FlatParent::Available(parent)
}

/// A borrowed, artefact-kind-agnostic view of an [`Archetype`]'s common fields
/// — the single access point the checks read through.
pub(crate) struct ArchetypeView<'a> {
    pub(crate) kind: ArtefactKind,
    pub(crate) archetype_id: &'a ArchetypeHrid,
    pub(crate) parent_archetype_id: Option<&'a str>,
    pub(crate) definition: &'a CComplexObject,
    pub(crate) terminology: &'a ArchetypeTerminology,
    pub(crate) rm_overlay: Option<&'a RmOverlay>,
    pub(crate) original_language: Option<&'a TerminologyCode>,
    pub(crate) description: Option<&'a ResourceDescription>,
    pub(crate) translations:
        Option<&'a BTreeMap<String, openehr_base::prelude::TranslationDetails>>,
    pub(crate) annotations: Option<&'a ResourceAnnotations>,
    pub(crate) adl_version: Option<&'a str>,
    pub(crate) rm_release: &'a str,
    pub(crate) is_differential: bool,
    /// The constituent terminologies by archetype id, on an operational
    /// template only (`OPT2/master03-opt_raw.adoc`: `component_terminologies`).
    pub(crate) components: Option<&'a BTreeMap<String, ArchetypeTerminology>>,
}

/// The `component_terminologies` of an operational template that carries none.
static NO_COMPONENTS: BTreeMap<String, ArchetypeTerminology> = BTreeMap::new();

impl ArchetypeView<'_> {
    /// True if this archetype specialises a parent.
    pub(crate) fn is_specialised(&self) -> bool {
        self.parent_archetype_id.is_some()
    }

    /// The archetype's specialisation level = the specialisation depth of its
    /// root node id (`master07` §Specialisation Depth; VARCN).
    pub(crate) fn specialisation_level(&self) -> usize {
        crate::codes::specialisation_depth(crate::aom::access::complex_node_id(self.definition))
            .unwrap_or(0)
    }
}

/// Build an [`ArchetypeView`] over any [`Archetype`] variant.
pub(crate) fn view(archetype: &Archetype) -> ArchetypeView<'_> {
    match archetype {
        Archetype::AuthoredArchetype(a) => match a.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => ArchetypeView {
                kind: ArtefactKind::Archetype,
                archetype_id: &d.archetype_id,
                parent_archetype_id: d.parent_archetype_id.as_deref(),
                definition: &d.definition,
                terminology: &d.terminology,
                rm_overlay: d.rm_overlay.as_ref(),
                original_language: Some(&d.original_language),
                description: d.description.as_deref(),
                translations: d.translations.as_ref(),
                annotations: d.annotations.as_ref(),
                adl_version: d.adl_version.as_deref(),
                rm_release: &d.rm_release,
                is_differential: d.is_differential,
                components: None,
            },
            AuthoredArchetype::Template(t) => ArchetypeView {
                kind: ArtefactKind::Template,
                archetype_id: &t.archetype_id,
                parent_archetype_id: t.parent_archetype_id.as_deref(),
                definition: &t.definition,
                terminology: &t.terminology,
                rm_overlay: t.rm_overlay.as_ref(),
                original_language: Some(&t.original_language),
                description: t.description.as_ref(),
                translations: t.translations.as_ref(),
                annotations: t.annotations.as_ref(),
                adl_version: t.adl_version.as_deref(),
                rm_release: &t.rm_release,
                is_differential: t.is_differential,
                components: None,
            },
            AuthoredArchetype::OperationalTemplate(o) => ArchetypeView {
                kind: ArtefactKind::OperationalTemplate,
                archetype_id: &o.archetype_id,
                parent_archetype_id: o.parent_archetype_id.as_deref(),
                definition: &o.definition,
                terminology: &o.terminology,
                rm_overlay: o.rm_overlay.as_ref(),
                original_language: Some(&o.original_language),
                description: o.description.as_ref(),
                translations: o.translations.as_ref(),
                annotations: o.annotations.as_ref(),
                adl_version: o.adl_version.as_deref(),
                rm_release: &o.rm_release,
                is_differential: o.is_differential,
                components: Some(o.component_terminologies.as_ref().unwrap_or(&NO_COMPONENTS)),
            },
        },
        Archetype::TemplateOverlay(t) => ArchetypeView {
            kind: ArtefactKind::TemplateOverlay,
            archetype_id: &t.archetype_id,
            parent_archetype_id: t.parent_archetype_id.as_deref(),
            definition: &t.definition,
            terminology: &t.terminology,
            rm_overlay: t.rm_overlay.as_ref(),
            original_language: None,
            description: None,
            translations: None,
            annotations: None,
            adl_version: None,
            rm_release: "",
            is_differential: t.is_differential,
            components: None,
        },
    }
}
