// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS priority category of each access, through a deployment-declared
//! map (`[audit.categories]`).
//!
//! Regulation (EU) 2025/327 Annex II 3.2(c) asks the European logging
//! component to record "the categories of data accessed", and Art. 14(1)
//! names the priority categories: patient summaries, electronic
//! prescriptions, electronic dispensations, medical imaging studies and
//! related imaging reports, medical test results, and discharge reports, with
//! national additional categories under its third subparagraph
//! (`docs/law/eu/ehds/text.html`). AQL selects archetypes and templates, never
//! categories, so a deployment declares which of its templates and archetypes
//! hold which category, and every access record is classified against that
//! map at the moment of access.
//!
//! **No openEHR spec governs this — our own design/extension.**
//!
//! Classification reads the template id first and the root archetype id
//! second, per served object. An object neither id classifies is recorded
//! `unclassified` with both ids as evidence and is never refused. `EHR`,
//! `EHR_STATUS`, the directory `FOLDER`, item tags and revision history are
//! classified by resource kind as holding no priority-category data. The map's
//! digest rides on every record, so a record says which map classified it.
//!
//! The classification is audit content: it is never written to the request
//! log, a span or a metric label.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The category spelling an access carries when the map cannot classify it.
pub const UNCLASSIFIED: &str = "unclassified";

/// The spelling of [`Category::None`].
const NONE: &str = "none";

/// The prefix of a national additional category (`national:<code>`).
const NATIONAL_PREFIX: &str = "national:";

/// One category a template or archetype is mapped to.
///
/// The six priority categories of EHDS Art. 14(1), a national additional
/// category (Art. 14(1) third subparagraph), or [`Category::None`] for content
/// that holds no priority-category data.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Category {
    /// `patient-summary` — Art. 14(1)(a).
    PatientSummary,
    /// `eprescription` — Art. 14(1)(b).
    Eprescription,
    /// `edispensation` — Art. 14(1)(c).
    Edispensation,
    /// `imaging` — Art. 14(1)(d), imaging studies and related reports.
    Imaging,
    /// `test-results` — Art. 14(1)(e), laboratory and other diagnostic results.
    TestResults,
    /// `discharge-report` — Art. 14(1)(f).
    DischargeReport,
    /// `national:<code>` — a category a Member State adds in national law.
    National(String),
    /// `none` — content mapped as holding no priority-category data.
    None,
}

/// A category spelling the map refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "`{0}` is not an audit category; the categories are patient-summary, eprescription, \
     edispensation, imaging, test-results, discharge-report, national:<code> and none"
)]
pub struct UnknownCategory(pub String);

impl Category {
    /// Parses one spelling.
    ///
    /// # Errors
    /// [`UnknownCategory`] for a spelling outside the set, including a
    /// `national:` code that is empty or carries whitespace or a comma.
    pub fn parse(spelling: &str) -> Result<Self, UnknownCategory> {
        Ok(match spelling {
            "patient-summary" => Category::PatientSummary,
            "eprescription" => Category::Eprescription,
            "edispensation" => Category::Edispensation,
            "imaging" => Category::Imaging,
            "test-results" => Category::TestResults,
            "discharge-report" => Category::DischargeReport,
            NONE => Category::None,
            other => match other.strip_prefix(NATIONAL_PREFIX) {
                Some(code)
                    if !code.is_empty()
                        && code.chars().all(|c| c.is_ascii_graphic() && c != ',') =>
                {
                    Category::National(code.to_owned())
                }
                _ => return Err(UnknownCategory(other.to_owned())),
            },
        })
    }

    /// The stored and configured spelling.
    #[must_use]
    pub fn spelling(&self) -> String {
        match self {
            Category::PatientSummary => "patient-summary".to_owned(),
            Category::Eprescription => "eprescription".to_owned(),
            Category::Edispensation => "edispensation".to_owned(),
            Category::Imaging => "imaging".to_owned(),
            Category::TestResults => "test-results".to_owned(),
            Category::DischargeReport => "discharge-report".to_owned(),
            Category::National(code) => format!("{NATIONAL_PREFIX}{code}"),
            Category::None => NONE.to_owned(),
        }
    }
}

impl TryFrom<String> for Category {
    type Error = UnknownCategory;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Category::parse(&value)
    }
}

impl From<Category> for String {
    fn from(category: Category) -> Self {
        category.spelling()
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.spelling())
    }
}

/// The `[audit.categories]` section: which categories each template and each
/// archetype holds.
///
/// Empty by default: FerroEHR ships no map, so every access is recorded
/// `unclassified` until a deployment declares one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CategoryMapConfig {
    /// Template id → categories (`[audit.categories.templates]`), consulted
    /// first.
    pub templates: BTreeMap<String, Vec<Category>>,
    /// Archetype id → categories (`[audit.categories.archetypes]`), consulted
    /// for an object whose template the map does not name.
    pub archetypes: BTreeMap<String, Vec<Category>>,
}

/// A map entry the boot validation refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CategoryMapError {
    /// A blank template or archetype key.
    #[error("audit.categories.{table} carries a blank key")]
    BlankKey {
        /// `templates` or `archetypes`.
        table: &'static str,
    },
    /// A key mapped to no category at all.
    #[error(
        "audit.categories.{table}.{key:?} maps to no category; map it to [\"none\"] when it holds \
         no priority-category data"
    )]
    Empty {
        /// `templates` or `archetypes`.
        table: &'static str,
        /// The key.
        key: String,
    },
    /// `none` combined with a category, which contradicts itself.
    #[error("audit.categories.{table}.{key:?} combines \"none\" with a category")]
    NoneWithCategory {
        /// `templates` or `archetypes`.
        table: &'static str,
        /// The key.
        key: String,
    },
    /// Two keys that differ only in letter case.
    #[error(
        "audit.categories.{table} names {key:?} twice, differing only in case; openEHR \
         identifiers compare case-insensitively"
    )]
    DuplicateKey {
        /// `templates` or `archetypes`.
        table: &'static str,
        /// The key.
        key: String,
    },
    /// An archetype key that is not an archetype HRID.
    #[error(
        "audit.categories.archetypes.{0:?} is not an archetype id \
         (openEHR-EHR-<CLASS>.<concept>.v<n>)"
    )]
    NotAnArchetypeId(String),
}

impl CategoryMapConfig {
    /// Validates the map and compiles it for lookup.
    ///
    /// # Errors
    /// Every [`CategoryMapError`] the map carries, so an operator fixes it in
    /// one pass.
    pub fn compile(&self) -> Result<CategoryMap, Vec<CategoryMapError>> {
        let mut errors = Vec::new();
        let templates = compile_table("templates", &self.templates, &mut errors);
        for key in self.archetypes.keys() {
            if !key.trim().is_empty() && key.parse::<openehr_base::prelude::ArchetypeId>().is_err()
            {
                errors.push(CategoryMapError::NotAnArchetypeId(key.clone()));
            }
        }
        let archetypes = compile_table("archetypes", &self.archetypes, &mut errors);
        if !errors.is_empty() {
            return Err(errors);
        }
        let digest = MapDigest::of(&templates, &archetypes);
        Ok(CategoryMap {
            templates,
            archetypes,
            digest,
        })
    }

    /// The boot validation's findings, as messages; empty when the map compiles.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        self.compile()
            .err()
            .unwrap_or_default()
            .iter()
            .map(ToString::to_string)
            .collect()
    }
}

/// Validates and case-folds one table of the map.
fn compile_table(
    table: &'static str,
    entries: &BTreeMap<String, Vec<Category>>,
    errors: &mut Vec<CategoryMapError>,
) -> BTreeMap<String, BTreeSet<Category>> {
    let mut out: BTreeMap<String, BTreeSet<Category>> = BTreeMap::new();
    for (key, categories) in entries {
        if key.trim().is_empty() {
            errors.push(CategoryMapError::BlankKey { table });
            continue;
        }
        if categories.is_empty() {
            errors.push(CategoryMapError::Empty {
                table,
                key: key.clone(),
            });
            continue;
        }
        let set: BTreeSet<Category> = categories.iter().cloned().collect();
        if set.contains(&Category::None) && set.len() > 1 {
            errors.push(CategoryMapError::NoneWithCategory {
                table,
                key: key.clone(),
            });
            continue;
        }
        if out.insert(fold(key), set).is_some() {
            errors.push(CategoryMapError::DuplicateKey {
                table,
                key: key.clone(),
            });
        }
    }
    out
}

/// The comparison form of an identifier: ASCII-lowercased, the form the node
/// table stores archetype ids in.
///
/// NOTE: BASE `base_types` master05 §Composite Identifiers and Case makes
/// identifier equality case-insensitive; template ids are compared the same way.
fn fold(id: &str) -> String {
    id.to_ascii_lowercase()
}

/// The SHA-256 digest of a compiled map's canonical form, so a record names
/// the map that classified it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MapDigest([u8; 32]);

impl MapDigest {
    /// The digest of the canonical rendering: one `kind\tkey\tcategory` line
    /// per mapping, keys case-folded, in sorted order.
    fn of(
        templates: &BTreeMap<String, BTreeSet<Category>>,
        archetypes: &BTreeMap<String, BTreeSet<Category>>,
    ) -> Self {
        let mut hasher = Sha256::new();
        for (kind, table) in [("template", templates), ("archetype", archetypes)] {
            for (key, categories) in table {
                for category in categories {
                    hasher.update(format!("{kind}\t{key}\t{category}\n").as_bytes());
                }
            }
        }
        MapDigest(hasher.finalize().into())
    }
}

impl fmt::Display for MapDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("sha256:")?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for MapDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Serialize for MapDigest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// A compiled, validated category map.
#[derive(Debug, Clone)]
pub struct CategoryMap {
    templates: BTreeMap<String, BTreeSet<Category>>,
    archetypes: BTreeMap<String, BTreeSet<Category>>,
    digest: MapDigest,
}

impl Default for CategoryMap {
    fn default() -> Self {
        let templates = BTreeMap::new();
        let archetypes = BTreeMap::new();
        let digest = MapDigest::of(&templates, &archetypes);
        Self {
            templates,
            archetypes,
            digest,
        }
    }
}

/// The resource kinds classified as holding no priority-category data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceKind {
    /// The `EHR` itself.
    Ehr,
    /// `EHR_STATUS`.
    EhrStatus,
    /// The EHR directory (`FOLDER`).
    Folder,
    /// `ITEM_TAG`s.
    ItemTag,
    /// A `REVISION_HISTORY`.
    RevisionHistory,
}

impl ResourceKind {
    /// The RM class name the record carries as evidence.
    #[must_use]
    pub fn rm_name(self) -> &'static str {
        match self {
            ResourceKind::Ehr => "EHR",
            ResourceKind::EhrStatus => "EHR_STATUS",
            ResourceKind::Folder => "FOLDER",
            ResourceKind::ItemTag => "ITEM_TAG",
            ResourceKind::RevisionHistory => "REVISION_HISTORY",
        }
    }
}

/// The identifiers of one object an access delivered, read or wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContentIds {
    /// The `template_id` the object was committed against.
    pub template_id: Option<String>,
    /// The archetype id of the object's root node.
    pub archetype_id: Option<String>,
    /// Set for an object of a kind that holds no priority-category data
    /// (`EHR_STATUS`, `FOLDER`), which is classified by kind alone.
    pub resource_kind: Option<ResourceKind>,
}

impl ContentIds {
    /// The ids of a versioned object of RM kind `kind`: an `EHR_STATUS` or a
    /// `FOLDER` is marked by kind, anything else carries its ids.
    #[must_use]
    pub fn of_kind(kind: &str, template_id: Option<String>, archetype_id: Option<String>) -> Self {
        let resource_kind = match kind {
            "EHR_STATUS" => Some(ResourceKind::EhrStatus),
            "FOLDER" => Some(ResourceKind::Folder),
            _ => None,
        };
        Self {
            template_id,
            archetype_id,
            resource_kind,
        }
    }
}

/// The positive archetype and template constraints of a bound AQL query.
///
/// Positive means reachable through `CONTAINS` and asserted rather than
/// negated: a `NOT CONTAINS` operand and a predicate under `NOT` never count.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct QueryConstraints {
    /// Template ids the query constrains to.
    pub template_ids: BTreeSet<String>,
    /// Archetype ids the query constrains to.
    pub archetype_ids: BTreeSet<String>,
    /// Whether a positive operand binds clinical content (anything but `EHR`,
    /// `EHR_STATUS` and `FOLDER`).
    pub reaches_content: bool,
    /// The no-category resource kinds the query reads, as evidence for a query
    /// that reaches no content.
    pub resource_kinds: BTreeSet<ResourceKind>,
}

/// What an access touched, as the classifier reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessedContent {
    /// The objects the access delivered, read or wrote.
    Objects(Vec<ContentIds>),
    /// A query answer that carries no object, classified from the query.
    Query(QueryConstraints),
    /// A resource kind that holds no priority-category data.
    ResourceKind(ResourceKind),
}

/// What a classification rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CategoryBasis {
    /// Every served object classified through its template id.
    Template,
    /// At least one object needed its root archetype id, whether or not that
    /// classified it.
    Archetype,
    /// The bound query's positive constraints.
    Query,
    /// The resource kind alone.
    ResourceKind,
}

impl CategoryBasis {
    /// The stored spelling, matching the `ck_audit_event_category_basis`
    /// constraint.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CategoryBasis::Template => "template",
            CategoryBasis::Archetype => "archetype",
            CategoryBasis::Query => "query",
            CategoryBasis::ResourceKind => "resource-kind",
        }
    }
}

/// The classification one access record carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRecord {
    /// The category spellings, sorted; `unclassified` when an object or query
    /// could not be classified.
    pub categories: Vec<String>,
    /// What the classification rests on.
    pub basis: CategoryBasis,
    /// The identifiers it rests on, sorted and capped at the record cap.
    pub evidence: Vec<String>,
}

/// The accumulator one classification fills.
#[derive(Default)]
struct Tally {
    categories: BTreeSet<String>,
    evidence: BTreeSet<String>,
    unclassified: bool,
}

impl Tally {
    fn add(&mut self, categories: &BTreeSet<Category>, evidence: &str) {
        self.categories
            .extend(categories.iter().map(Category::spelling));
        self.evidence.insert(evidence.to_owned());
    }

    fn finish(self, basis: CategoryBasis) -> CategoryRecord {
        let mut categories = self.categories;
        if self.unclassified {
            categories.insert(UNCLASSIFIED.to_owned());
        }
        // `none` says nothing beside a real category or an unknown one.
        if categories.iter().any(|c| c != NONE) {
            categories.remove(NONE);
        }
        let mut evidence: Vec<String> = self.evidence.into_iter().collect();
        evidence.truncate(crate::versioning::origins::RECORD_CAP);
        CategoryRecord {
            categories: categories.into_iter().collect(),
            basis,
            evidence,
        }
    }
}

impl CategoryMap {
    /// The digest of this map's canonical form.
    #[must_use]
    pub fn digest(&self) -> MapDigest {
        self.digest
    }

    /// Whether the map declares nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.templates.is_empty() && self.archetypes.is_empty()
    }

    /// Every mapping as `(key kind, case-folded key, category)`, the rows the
    /// boot mirror writes for the retention register.
    #[must_use]
    pub fn rows(&self) -> Vec<(&'static str, String, String)> {
        let mut rows = Vec::new();
        for (kind, table) in [
            ("template", &self.templates),
            ("archetype", &self.archetypes),
        ] {
            for (key, categories) in table {
                for category in categories {
                    rows.push((kind, key.clone(), category.spelling()));
                }
            }
        }
        rows
    }

    fn template(&self, id: &str) -> Option<&BTreeSet<Category>> {
        self.templates.get(&fold(id))
    }

    fn archetype(&self, id: &str) -> Option<&BTreeSet<Category>> {
        self.archetypes.get(&fold(id))
    }

    /// Classifies one access.
    ///
    /// Objects are read template first, then root archetype; an object neither
    /// id classifies adds `unclassified` and both ids as evidence. A query
    /// answer with no object reads the query's template constraints first, then
    /// its archetype constraints, and is `unclassified` when none of them is
    /// mapped.
    #[must_use]
    pub fn classify(&self, content: &AccessedContent) -> CategoryRecord {
        match content {
            AccessedContent::Objects(objects) => self.classify_objects(objects),
            AccessedContent::Query(constraints) => self.classify_query(constraints),
            AccessedContent::ResourceKind(kind) => {
                let mut tally = Tally::default();
                tally.categories.insert(NONE.to_owned());
                tally.evidence.insert(kind.rm_name().to_owned());
                tally.finish(CategoryBasis::ResourceKind)
            }
        }
    }

    fn classify_objects(&self, objects: &[ContentIds]) -> CategoryRecord {
        let mut tally = Tally::default();
        let mut by_archetype = false;
        let mut by_template = false;
        for object in objects {
            if let Some(kind) = object.resource_kind {
                tally.categories.insert(NONE.to_owned());
                tally.evidence.insert(kind.rm_name().to_owned());
                continue;
            }
            if let Some(template) = object.template_id.as_deref()
                && let Some(categories) = self.template(template)
            {
                tally.add(categories, template);
                by_template = true;
                continue;
            }
            by_archetype = true;
            if let Some(archetype) = object.archetype_id.as_deref()
                && let Some(categories) = self.archetype(archetype)
            {
                tally.add(categories, archetype);
                continue;
            }
            tally.unclassified = true;
            tally.evidence.extend(object.template_id.iter().cloned());
            tally.evidence.extend(object.archetype_id.iter().cloned());
        }
        if objects.is_empty() {
            tally.unclassified = true;
        }
        let basis = if by_archetype {
            CategoryBasis::Archetype
        } else if by_template || objects.is_empty() {
            CategoryBasis::Template
        } else {
            CategoryBasis::ResourceKind
        };
        tally.finish(basis)
    }

    fn classify_query(&self, constraints: &QueryConstraints) -> CategoryRecord {
        let mut tally = Tally::default();
        if !constraints.reaches_content {
            tally.categories.insert(NONE.to_owned());
            tally.evidence.extend(
                constraints
                    .resource_kinds
                    .iter()
                    .map(|k| k.rm_name().to_owned()),
            );
            return tally.finish(CategoryBasis::ResourceKind);
        }
        for template in &constraints.template_ids {
            if let Some(categories) = self.template(template) {
                tally.add(categories, template);
            }
        }
        if tally.categories.is_empty() {
            for archetype in &constraints.archetype_ids {
                if let Some(categories) = self.archetype(archetype) {
                    tally.add(categories, archetype);
                }
            }
        }
        if tally.categories.is_empty() {
            tally.unclassified = true;
            tally
                .evidence
                .extend(constraints.template_ids.iter().cloned());
            tally
                .evidence
                .extend(constraints.archetype_ids.iter().cloned());
        }
        tally.finish(CategoryBasis::Query)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(toml_text: &str) -> CategoryMap {
        let config: CategoryMapConfig = toml::from_str(toml_text).expect("parse");
        config.compile().expect("compile")
    }

    fn object(template: Option<&str>, archetype: Option<&str>) -> ContentIds {
        ContentIds {
            template_id: template.map(str::to_owned),
            archetype_id: archetype.map(str::to_owned),
            resource_kind: None,
        }
    }

    const MAP: &str = r#"
        [templates]
        "Lab Report" = ["test-results"]
        "IPS" = ["patient-summary"]
        "Admin Note" = ["none"]
        [archetypes]
        "openEHR-EHR-COMPOSITION.imaging_report.v1" = ["imaging", "national:nl-beeld"]
    "#;

    #[test]
    fn every_spelling_round_trips_and_unknown_ones_are_refused() {
        for spelling in [
            "patient-summary",
            "eprescription",
            "edispensation",
            "imaging",
            "test-results",
            "discharge-report",
            "national:de-mio",
            "none",
        ] {
            let category = Category::parse(spelling).expect(spelling);
            assert_eq!(category.spelling(), spelling);
        }
        for bad in [
            "laboratory",
            "national:",
            "national:a b",
            "unclassified",
            "",
        ] {
            assert!(Category::parse(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn the_boot_validation_names_every_defect() {
        let config: CategoryMapConfig = toml::from_str(
            r#"
            [templates]
            "a" = []
            "b" = ["none", "imaging"]
            "C" = ["imaging"]
            "c" = ["imaging"]
            [archetypes]
            "not-an-archetype" = ["imaging"]
            "#,
        )
        .expect("parse");
        let errors = config.errors();
        assert_eq!(errors.len(), 4, "{errors:#?}");
        assert!(errors.iter().any(|e| e.contains("maps to no category")));
        assert!(errors.iter().any(|e| e.contains("combines \"none\"")));
        assert!(errors.iter().any(|e| e.contains("differing only in case")));
        assert!(errors.iter().any(|e| e.contains("is not an archetype id")));
        let unknown = toml::from_str::<CategoryMapConfig>("[templates]\n\"a\" = [\"lab\"]")
            .expect_err("an unknown spelling fails to load");
        assert!(unknown.to_string().contains("is not an audit category"));
    }

    #[test]
    fn the_digest_is_stable_over_case_and_order_and_moves_with_the_content() {
        let a = map(MAP);
        let b = map(r#"
            [archetypes]
            "openEHR-EHR-COMPOSITION.IMAGING_REPORT.v1" = ["national:nl-beeld", "imaging"]
            [templates]
            "admin note" = ["none"]
            "ips" = ["patient-summary"]
            "lab report" = ["test-results"]
            "#);
        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.digest(), CategoryMap::default().digest());
        assert!(a.digest().to_string().starts_with("sha256:"));
    }

    #[test]
    fn objects_are_classified_template_first_then_root_archetype() {
        let m = map(MAP);
        let by_template = m.classify(&AccessedContent::Objects(vec![object(
            Some("lab report"),
            Some("openEHR-EHR-COMPOSITION.imaging_report.v1"),
        )]));
        assert_eq!(by_template.categories, ["test-results"]);
        assert_eq!(by_template.basis, CategoryBasis::Template);
        assert_eq!(by_template.evidence, ["lab report"]);

        let by_archetype = m.classify(&AccessedContent::Objects(vec![object(
            Some("unmapped"),
            Some("openehr-ehr-composition.imaging_report.v1"),
        )]));
        assert_eq!(by_archetype.categories, ["imaging", "national:nl-beeld"]);
        assert_eq!(by_archetype.basis, CategoryBasis::Archetype);
    }

    #[test]
    fn an_unmapped_object_is_unclassified_with_its_ids_as_evidence() {
        let m = map(MAP);
        let record = m.classify(&AccessedContent::Objects(vec![
            object(Some("IPS"), None),
            object(Some("Vitals"), Some("openEHR-EHR-COMPOSITION.encounter.v1")),
        ]));
        assert_eq!(record.categories, ["patient-summary", UNCLASSIFIED]);
        assert_eq!(
            record.evidence,
            ["IPS", "Vitals", "openEHR-EHR-COMPOSITION.encounter.v1"]
        );
        assert_eq!(record.basis, CategoryBasis::Archetype);
    }

    #[test]
    fn none_gives_way_to_a_real_category_and_kinds_classify_as_none() {
        let m = map(MAP);
        let mixed = m.classify(&AccessedContent::Objects(vec![
            object(Some("Admin Note"), None),
            object(Some("Lab Report"), None),
            ContentIds::of_kind("EHR_STATUS", None, None),
        ]));
        assert_eq!(mixed.categories, ["test-results"]);
        let kind = m.classify(&AccessedContent::ResourceKind(ResourceKind::Ehr));
        assert_eq!(kind.categories, ["none"]);
        assert_eq!(kind.basis, CategoryBasis::ResourceKind);
        assert_eq!(kind.evidence, ["EHR"]);
    }

    #[test]
    fn a_query_without_objects_reads_its_positive_constraints() {
        let m = map(MAP);
        let constraints = QueryConstraints {
            template_ids: BTreeSet::new(),
            archetype_ids: BTreeSet::from([
                "openEHR-EHR-COMPOSITION.imaging_report.v1".to_owned(),
                "openEHR-EHR-OBSERVATION.unmapped.v1".to_owned(),
            ]),
            reaches_content: true,
            resource_kinds: BTreeSet::new(),
        };
        let record = m.classify(&AccessedContent::Query(constraints));
        assert_eq!(record.categories, ["imaging", "national:nl-beeld"]);
        assert_eq!(record.basis, CategoryBasis::Query);

        let none_mapped = m.classify(&AccessedContent::Query(QueryConstraints {
            template_ids: BTreeSet::from(["Vitals".to_owned()]),
            reaches_content: true,
            ..QueryConstraints::default()
        }));
        assert_eq!(none_mapped.categories, [UNCLASSIFIED]);
        assert_eq!(none_mapped.evidence, ["Vitals"]);

        let ehr_only = m.classify(&AccessedContent::Query(QueryConstraints {
            resource_kinds: BTreeSet::from([ResourceKind::Ehr]),
            ..QueryConstraints::default()
        }));
        assert_eq!(ehr_only.categories, ["none"]);
        assert_eq!(ehr_only.basis, CategoryBasis::ResourceKind);
    }
}
