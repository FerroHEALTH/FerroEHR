// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS category of each access: the deployment's map
//! (`[audit.categories]`) and what FerroEHR reads off its own data for
//! `ehds-logging` to classify.
//!
//! Regulation (EU) 2025/327 Annex II 3.2(c) asks the European logging
//! component to record "the categories of data accessed", and Art. 14(1)
//! names the priority categories (`docs/law/eu/ehds/text.html`). The shared
//! crate `ehds-logging` (FerroAUDIT) holds the categories, the map and the
//! classification; FerroEHR keeps what the crate leaves to the product: the
//! map's configuration, the openEHR checks on its keys, the digest that names
//! it, and the model ids of what an access reached.
//!
//! **No openEHR spec governs this — our own design/extension.**
//!
//! An object the map cannot classify is recorded `unclassified` and is never
//! refused. `EHR`, `EHR_STATUS`, `EHR_ACCESS`, the directory `FOLDER`, item
//! tags and revision history hold no data of any category. The classification
//! is audit content: it is never written to the request log, a span or a
//! metric label.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ehds_logging::category::Category;
use ehds_logging::classify::{Basis, Classification, Evidence, Queried, RootObject};
use ehds_logging::map::{CategoryMap, Declared, MapError, Mapping, Matching};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The word a stored record carries for an access the map cannot classify.
pub const UNCLASSIFIED: &str = "unclassified";

/// The word a stored record carries for an access of no category, the word a
/// map declares it with.
pub const NONE: &str = ehds_logging::map::NONE;

/// The `[audit.categories]` section: the national categories the deployment
/// adds, and which categories each template and each archetype holds.
///
/// A category is a priority category of Art. 14(1) by its code in HL7
/// Europe's `EEHRxFDocumentPriorityCategoryCS` (`Patient-Summaries`,
/// `Electronic-Prescriptions`, `Electronic-Dispensations`, `Medical-Imaging`,
/// `Laboratory-Reports`, `Discharge-Reports`) or as `<system>|<code>`, or a
/// national one declared in `national` as `<system>|<code>`; a key whose data
/// hold no category maps to the word `"none"`. Empty by default: FerroEHR
/// ships no map, so every access is recorded `unclassified` until a deployment
/// declares one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CategoryMapConfig {
    /// The national categories (Art. 14(1) third subparagraph), each
    /// `<system>|<code>`.
    pub national: Vec<String>,
    /// Template id → categories (`[audit.categories.templates]`), consulted
    /// first.
    pub templates: BTreeMap<String, Declared>,
    /// Archetype id → categories (`[audit.categories.archetypes]`), consulted
    /// for an object whose template the map does not name.
    pub archetypes: BTreeMap<String, Declared>,
}

/// A map entry the boot validation refuses.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CategoryMapError {
    /// `ehds-logging` refused a declaration.
    #[error(transparent)]
    Map(MapError),
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

impl From<MapError> for CategoryMapError {
    fn from(error: MapError) -> Self {
        match error {
            MapError::DuplicateKey { table, second, .. } => CategoryMapError::DuplicateKey {
                table: table.name(),
                key: second,
            },
            MapError::ArchetypeId { key } => CategoryMapError::NotAnArchetypeId(key),
            other => CategoryMapError::Map(other),
        }
    }
}

impl CategoryMapConfig {
    /// Validates the map and compiles it for classification.
    ///
    /// Every key is checked on its own, so one pass names every defect: the
    /// openEHR form of an archetype key, two keys that differ only in case,
    /// and each declaration `ehds-logging` refuses.
    ///
    /// # Errors
    /// Every [`CategoryMapError`] the map carries.
    pub fn compile(&self) -> Result<Classifier, Vec<CategoryMapError>> {
        // NOTE: openEHR BASE 1.1.0 Base Types, Composite Identifiers and Case: ids compare
        // case-insensitively and keep their case, which ehds-logging's OpenEhr matching does.
        let map = CategoryMap::declare_with(
            Matching::OpenEhr,
            &self.national,
            &self.templates,
            &self.archetypes,
        )
        .map_err(|errors| {
            errors
                .into_iter()
                .map(CategoryMapError::from)
                .collect::<Vec<_>>()
        })?
        .with_evidence_limit(crate::versioning::origins::RECORD_CAP);
        let digest = MapDigest::of(&map);
        let mut rows = Vec::new();
        for (kind, keys) in [
            ("template", map.templates().collect::<Vec<_>>()),
            ("archetype", map.archetypes().collect::<Vec<_>>()),
        ] {
            for (key, mapping) in keys {
                let key = fold(key);
                match mapping {
                    Mapping::NoCategory => rows.push((kind, key, NONE.to_owned())),
                    Mapping::Categories(categories) => rows.extend(
                        categories
                            .iter()
                            .map(|category| (kind, key.clone(), category.token())),
                    ),
                }
            }
        }
        rows.sort();
        rows.dedup();
        Ok(Classifier {
            map: map.with_digest(digest.to_string()),
            digest,
            rows,
        })
    }

    /// The boot validation's findings, as messages; empty when the map compiles.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        match self.compile() {
            Ok(_) => Vec::new(),
            Err(errors) => errors.iter().map(ToString::to_string).collect(),
        }
    }
}

/// The comparison form of an identifier, which the retention register's
/// mirror rows store: ASCII-lowercased, the form the node table stores
/// archetype ids in (openEHR BASE 1.1.0, Composite Identifiers and Case).
fn fold(id: &str) -> String {
    id.to_ascii_lowercase()
}

/// The SHA-256 digest of a compiled map's canonical text, so a record names the
/// map that classified it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct MapDigest([u8; 32]);

impl MapDigest {
    /// The digest of `map`'s canonical text ([`CategoryMap::canonical`]).
    fn of(map: &CategoryMap) -> Self {
        MapDigest(Sha256::digest(map.canonical().as_bytes()).into())
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

impl std::str::FromStr for MapDigest {
    type Err = MapDigestError;

    /// Reads the `sha256:<hex>` text [`fmt::Display`] writes.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let hex = text.strip_prefix("sha256:").ok_or(MapDigestError)?;
        let mut digest = [0_u8; 32];
        if hex.len() != 64 || !hex.is_ascii() {
            return Err(MapDigestError);
        }
        for (byte, pair) in digest.iter_mut().zip(hex.as_bytes().as_chunks::<2>().0) {
            let pair = std::str::from_utf8(pair).map_err(|_not_text| MapDigestError)?;
            *byte = u8::from_str_radix(pair, 16).map_err(|_not_hex| MapDigestError)?;
        }
        Ok(MapDigest(digest))
    }
}

/// A text that is no `sha256:<64 hex digits>` map digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("a category map digest is `sha256:` and 64 hex digits")]
pub struct MapDigestError;

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

/// The compiled map every access record is classified through.
#[derive(Debug, Clone)]
pub struct Classifier {
    map: CategoryMap,
    digest: MapDigest,
    rows: Vec<(&'static str, String, String)>,
}

impl Default for Classifier {
    fn default() -> Self {
        let map = CategoryMap::default();
        let digest = MapDigest::of(&map);
        Self {
            map: map.with_digest(digest.to_string()),
            digest,
            rows: Vec::new(),
        }
    }
}

/// The resource kinds that hold no data of any category, as `ehds-logging`
/// names them.
pub type ResourceKind = ehds_logging::classify::ResourceKind;

/// The identifiers of one object an access delivered, read or wrote.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(into = "StoredIds", from = "StoredIds")]
pub struct ContentIds {
    /// The `template_id` the object was committed against.
    pub template_id: Option<String>,
    /// The archetype id of the object's root node.
    pub archetype_id: Option<String>,
    /// Set for an object of a kind that holds no data of any category
    /// (`EHR_STATUS`, `EHR_ACCESS`, `FOLDER`), which is classified by kind
    /// alone.
    pub resource_kind: Option<ResourceKind>,
}

/// The serialized form of [`ContentIds`]: the kind by its code.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredIds {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    archetype_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resource_kind: Option<String>,
}

impl From<ContentIds> for StoredIds {
    fn from(ids: ContentIds) -> Self {
        Self {
            template_id: ids.template_id,
            archetype_id: ids.archetype_id,
            resource_kind: ids.resource_kind.map(|kind| kind.code().to_owned()),
        }
    }
}

impl From<StoredIds> for ContentIds {
    fn from(ids: StoredIds) -> Self {
        Self {
            template_id: ids.template_id,
            archetype_id: ids.archetype_id,
            resource_kind: ids.resource_kind.as_deref().map(kind_code::of),
        }
    }
}

impl ContentIds {
    /// The ids of a versioned object of RM kind `kind`: an `EHR_STATUS`, an
    /// `EHR_ACCESS` or a `FOLDER` is marked by kind, anything else carries its
    /// ids.
    #[must_use]
    pub fn of_kind(kind: &str, template_id: Option<String>, archetype_id: Option<String>) -> Self {
        let resource_kind = match kind {
            "EHR_STATUS" => Some(ResourceKind::EhrStatus),
            "EHR_ACCESS" => Some(ResourceKind::EhrAccess),
            "FOLDER" => Some(ResourceKind::Folder),
            _ => None,
        };
        Self {
            template_id,
            archetype_id,
            resource_kind,
        }
    }

    /// The root object `ehds-logging` classifies, its ids in comparison form.
    fn root_object(&self) -> RootObject {
        RootObject {
            template_id: self.template_id.clone(),
            archetype_id: self.archetype_id.clone(),
            version_uid: None,
            kind: self.resource_kind.clone(),
        }
    }
}

/// The positive archetype and template constraints of a bound AQL query.
///
/// Positive means reachable through `CONTAINS` and asserted rather than
/// negated: a `NOT CONTAINS` operand and a predicate under `NOT` never count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueryConstraints {
    /// Template ids the query constrains to.
    pub template_ids: BTreeSet<String>,
    /// Archetype ids the query constrains to.
    pub archetype_ids: BTreeSet<String>,
    /// Whether a positive operand binds clinical content (anything but `EHR`,
    /// `EHR_STATUS`, `EHR_ACCESS` and `FOLDER`).
    pub reaches_content: bool,
    /// Whether every positive operand that binds clinical content is bound to
    /// an archetype id or a template id, so what the query read is known.
    pub every_root_bound: bool,
    /// The no-category resource kinds the query reads, as evidence for a query
    /// that reaches no content.
    #[serde(with = "kind_code::set")]
    pub resource_kinds: BTreeSet<ResourceKind>,
}

/// What an access touched, as FerroEHR reads it off its own data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum AccessedContent {
    /// The objects the access delivered, read or wrote.
    Objects(Vec<ContentIds>),
    /// A query answer that carries no object, classified from the query.
    Query(QueryConstraints),
    /// A resource kind that holds no data of any category.
    ResourceKind(#[serde(with = "kind_code")] ResourceKind),
}

/// What one classification read, and whether the access wrote it: the input
/// [`Classifier::classify`] takes, which the audit store keeps so the
/// classification is computed again when a record is rendered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassifiedContent {
    /// What the access touched.
    pub content: AccessedContent,
    /// Whether the access wrote its objects rather than read them.
    pub written: bool,
}

/// The serialized form of a [`ResourceKind`]: its code, which
/// [`ResourceKind::code`] writes and every record names it by.
mod kind_code {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::ResourceKind;

    /// The kind a code names; a code no listed kind writes is a kind the
    /// deployment names.
    pub(super) fn of(code: &str) -> ResourceKind {
        match code {
            "EHR" => ResourceKind::Ehr,
            "EHR_STATUS" => ResourceKind::EhrStatus,
            "EHR_ACCESS" => ResourceKind::EhrAccess,
            "FOLDER" => ResourceKind::Folder,
            "ITEM_TAG" => ResourceKind::ItemTag,
            "REVISION_HISTORY" => ResourceKind::RevisionHistory,
            other => ResourceKind::Other(other.to_owned()),
        }
    }

    pub(super) fn serialize<S: Serializer>(kind: &ResourceKind, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(kind.code())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ResourceKind, D::Error> {
        String::deserialize(d).map(|code| of(&code))
    }

    pub(super) mod set {
        use std::collections::BTreeSet;

        use serde::{Deserialize, Deserializer, Serializer};

        use super::ResourceKind;

        pub(in super::super) fn serialize<S: Serializer>(
            kinds: &BTreeSet<ResourceKind>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            s.collect_seq(kinds.iter().map(ResourceKind::code))
        }

        pub(in super::super) fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<BTreeSet<ResourceKind>, D::Error> {
            Vec::<String>::deserialize(d)
                .map(|codes| codes.iter().map(|code| super::of(code)).collect())
        }
    }

    #[cfg(test)]
    mod tests {
        use std::collections::BTreeSet;

        use super::{ResourceKind, of};

        #[test]
        fn every_listed_kind_reads_back_from_its_code() {
            let kinds = BTreeSet::from([
                ResourceKind::Ehr,
                ResourceKind::EhrStatus,
                ResourceKind::EhrAccess,
                ResourceKind::Folder,
                ResourceKind::ItemTag,
                ResourceKind::RevisionHistory,
                ResourceKind::Other("CUSTOM".to_owned()),
            ]);
            for kind in kinds {
                assert_eq!(of(kind.code()), kind);
            }
        }
    }
}

/// What a stored classification was read from, as `ehds-logging` names it;
/// its code is the stored spelling the `ck_audit_event_category_basis`
/// constraint admits.
pub type ReadFrom = Basis;

/// The classification one access record carries: what `ehds-logging` made of
/// it, and what FerroEHR read it from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CategoryRecord {
    /// The categories, their bases and the evidence, as `ehds-logging`
    /// classified them.
    pub classification: Classification,
    /// What the classification was read from.
    pub read_from: ReadFrom,
    /// What the classifier was given, from which the same map classifies the
    /// same record again.
    pub content: ClassifiedContent,
}

impl CategoryRecord {
    /// The stored category words, sorted: each category as `<system>|<code>`,
    /// `none` for an access of no category, `unclassified` when the map could
    /// not classify all of it.
    #[must_use]
    pub fn stored_categories(&self) -> Vec<String> {
        let mut words: BTreeSet<String> = self
            .classification
            .categories()
            .keys()
            .map(Category::token)
            .collect();
        if self.classification.is_no_category() {
            words.insert(NONE.to_owned());
        }
        if self.classification.unclassified().is_some() {
            words.insert(UNCLASSIFIED.to_owned());
        }
        words.into_iter().collect()
    }

    /// The stored evidence, sorted and capped at the record cap: the template
    /// and archetype ids the classification read, as the access named them,
    /// and the RM class names of the resource kinds of no category.
    #[must_use]
    pub fn stored_evidence(&self) -> Vec<String> {
        let mut evidence: BTreeSet<String> = self
            .classification
            .templates()
            .iter()
            .chain(self.classification.archetypes())
            .cloned()
            .collect();
        evidence.extend(
            self.classification
                .no_category_kinds()
                .iter()
                .map(|kind| kind.code().to_owned()),
        );
        let mut evidence: Vec<String> = evidence.into_iter().collect();
        evidence.truncate(crate::versioning::origins::RECORD_CAP);
        evidence
    }
}

impl Classifier {
    /// The digest of this map's canonical text.
    #[must_use]
    pub fn digest(&self) -> MapDigest {
        self.digest
    }

    /// The compiled `ehds-logging` map.
    #[must_use]
    pub fn map(&self) -> &CategoryMap {
        &self.map
    }

    /// Whether the map declares nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// The category `reference` names, as `<system>|<code>`: a priority
    /// category by its bare code or its token, or a national one the map
    /// declares.
    #[must_use]
    pub fn category_token(&self, reference: &str) -> Option<String> {
        self.map
            .category(reference)
            .map(|category| category.token())
    }

    /// Every mapping as `(key kind, case-folded key, category)`, the rows the
    /// boot mirror writes for the retention register; a key of no category
    /// maps to `none`, every other to a `<system>|<code>` token.
    #[must_use]
    pub fn rows(&self) -> &[(&'static str, String, String)] {
        &self.rows
    }

    /// Classifies one access whose objects were read (`written` false) or
    /// written (`written` true).
    ///
    /// Objects are read template first, then root archetype; an object of a
    /// kind that holds no category data adds only its kind as evidence. A
    /// query answer with no object is classified from the query's positive
    /// constraints.
    #[must_use]
    pub fn classify(&self, content: &AccessedContent, written: bool) -> CategoryRecord {
        let kinds_only = |kinds: Vec<ResourceKind>| {
            Evidence::reached(
                Basis::ResourceKind,
                kinds
                    .into_iter()
                    .map(|kind| RootObject {
                        kind: Some(kind),
                        ..RootObject::default()
                    })
                    .collect(),
            )
        };
        let (evidence, read_from) = match content {
            AccessedContent::ResourceKind(kind) => {
                (Evidence::resource(kind.clone()), Basis::ResourceKind)
            }
            AccessedContent::Objects(objects) => {
                let basis = if written {
                    Basis::Written
                } else {
                    Basis::Returned
                };
                let content = objects.iter().any(|object| object.resource_kind.is_none());
                let roots: Vec<RootObject> = objects.iter().map(ContentIds::root_object).collect();
                if content || roots.is_empty() {
                    (Evidence::reached(basis, roots), basis)
                } else {
                    (
                        Evidence::reached(Basis::ResourceKind, roots),
                        Basis::ResourceKind,
                    )
                }
            }
            AccessedContent::Query(constraints) => {
                if constraints.reaches_content {
                    let queried = Queried {
                        templates: constraints.template_ids.clone(),
                        archetypes: constraints.archetype_ids.clone(),
                        every_root_bound: constraints.every_root_bound,
                    };
                    (
                        Evidence::reached(Basis::Returned, Vec::new()).queried(queried),
                        Basis::Queried,
                    )
                } else if constraints.resource_kinds.is_empty() {
                    (Evidence::no_category(), Basis::ResourceKind)
                } else {
                    (
                        kinds_only(constraints.resource_kinds.iter().cloned().collect()),
                        Basis::ResourceKind,
                    )
                }
            }
        };
        CategoryRecord {
            classification: self.map.classify(&evidence),
            read_from,
            content: ClassifiedContent {
                content: content.clone(),
                written,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAB: &str = "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|\
         Laboratory-Reports";
    const SUMMARY: &str = "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|\
         Patient-Summaries";
    const IMAGING: &str = "http://hl7.eu/fhir/health-data-api/CodeSystem/eehrxf-document-priority-category-cs|\
         Medical-Imaging";
    const BEELD: &str = "urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld";

    fn map(toml_text: &str) -> Classifier {
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
        national = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]
        [templates]
        "Lab Report" = ["Laboratory-Reports"]
        "IPS" = ["Patient-Summaries"]
        "Admin Note" = "none"
        [archetypes]
        "openEHR-EHR-COMPOSITION.imaging_report.v1" = ["Medical-Imaging", "urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]
    "#;

    #[test]
    fn the_config_round_trips_through_its_serialized_form() {
        let config: CategoryMapConfig = toml::from_str(MAP).expect("parse");
        let text = toml::to_string(&config).expect("serialize");
        let again: CategoryMapConfig = toml::from_str(&text).expect("parse again");
        assert_eq!(config, again);
    }

    #[test]
    fn the_boot_validation_names_every_defect() {
        let config: CategoryMapConfig = toml::from_str(
            r#"
            [templates]
            "a" = []
            "b" = ["none", "Medical-Imaging"]
            "C" = ["Medical-Imaging"]
            "c" = ["Medical-Imaging"]
            [archetypes]
            "not-an-archetype" = ["Medical-Imaging"]
            "#,
        )
        .expect("parse");
        let errors = config.errors();
        assert_eq!(errors.len(), 4, "{errors:#?}");
        assert!(errors.iter().any(|e| e.contains("maps to no category")));
        assert!(errors.iter().any(|e| e.contains("\"none\"")));
        assert!(errors.iter().any(|e| e.contains("differing only in case")));
        assert!(errors.iter().any(|e| e.contains("is not an archetype id")));
        let unknown: CategoryMapConfig =
            toml::from_str("[templates]\n\"a\" = [\"lab\"]").expect("parse");
        assert!(
            unknown
                .errors()
                .iter()
                .any(|e| e.contains("neither a priority category nor a declared national one")),
            "an unknown category is refused"
        );
    }

    #[test]
    fn the_digest_is_stable_over_case_and_order_and_moves_with_the_content() {
        let a = map(MAP);
        let b = map(r#"
            national = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld"]
            [archetypes]
            "openEHR-EHR-COMPOSITION.IMAGING_REPORT.v1" = ["urn:oid:2.16.840.1.113883.2.4.3.11.60.1|beeld", "Medical-Imaging"]
            [templates]
            "admin note" = "none"
            "ips" = ["Patient-Summaries"]
            "lab report" = ["Laboratory-Reports"]
            "#);
        assert_eq!(a.digest(), b.digest());
        assert_ne!(a.digest(), Classifier::default().digest());
        assert!(a.digest().to_string().starts_with("sha256:"));
    }

    #[test]
    fn objects_are_classified_template_first_then_root_archetype() {
        let m = map(MAP);
        let by_template = m.classify(
            &AccessedContent::Objects(vec![object(
                Some("lab report"),
                Some("openEHR-EHR-COMPOSITION.imaging_report.v1"),
            )]),
            false,
        );
        assert_eq!(by_template.stored_categories(), [LAB]);
        assert_eq!(by_template.read_from, ReadFrom::Returned);

        let by_archetype = m.classify(
            &AccessedContent::Objects(vec![object(
                Some("unmapped"),
                Some("openehr-ehr-composition.imaging_report.v1"),
            )]),
            true,
        );
        assert_eq!(by_archetype.stored_categories(), [IMAGING, BEELD]);
        assert_eq!(by_archetype.read_from, ReadFrom::Written);
    }

    #[test]
    fn an_unmapped_object_is_unclassified_with_its_ids_as_evidence() {
        let m = map(MAP);
        let record = m.classify(
            &AccessedContent::Objects(vec![
                object(Some("IPS"), None),
                object(Some("Vitals"), Some("openEHR-EHR-COMPOSITION.encounter.v1")),
            ]),
            false,
        );
        assert_eq!(record.stored_categories(), [SUMMARY, UNCLASSIFIED]);
        // NOTE: openEHR BASE 1.1.0, Composite Identifiers and Case: ids keep their case.
        assert_eq!(
            record.stored_evidence(),
            ["IPS", "Vitals", "openEHR-EHR-COMPOSITION.encounter.v1"]
        );
    }

    #[test]
    fn none_gives_way_to_a_real_category_and_kinds_classify_as_none() {
        let m = map(MAP);
        let mixed = m.classify(
            &AccessedContent::Objects(vec![
                object(Some("Admin Note"), None),
                object(Some("Lab Report"), None),
                ContentIds::of_kind("EHR_STATUS", None, None),
            ]),
            false,
        );
        assert_eq!(mixed.stored_categories(), [LAB]);
        let kind = m.classify(&AccessedContent::ResourceKind(ResourceKind::Ehr), false);
        assert_eq!(kind.stored_categories(), [NONE]);
        assert_eq!(kind.read_from, ReadFrom::ResourceKind);
        assert_eq!(kind.stored_evidence(), ["EHR"]);
    }

    /// Every queried id must map: an unmapped one leaves the access
    /// unclassified beside what did map (Annex II 3.2(c) records the
    /// categories of all the data accessed).
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
            every_root_bound: true,
            resource_kinds: BTreeSet::new(),
        };
        let record = m.classify(&AccessedContent::Query(constraints), false);
        assert_eq!(record.stored_categories(), [IMAGING, UNCLASSIFIED, BEELD]);
        assert_eq!(record.read_from, ReadFrom::Queried);

        let mapped = m.classify(
            &AccessedContent::Query(QueryConstraints {
                archetype_ids: BTreeSet::from([
                    "openEHR-EHR-COMPOSITION.imaging_report.v1".to_owned()
                ]),
                reaches_content: true,
                every_root_bound: true,
                ..QueryConstraints::default()
            }),
            false,
        );
        assert_eq!(mapped.stored_categories(), [IMAGING, BEELD]);

        let none_mapped = m.classify(
            &AccessedContent::Query(QueryConstraints {
                template_ids: BTreeSet::from(["Vitals".to_owned()]),
                reaches_content: true,
                every_root_bound: true,
                ..QueryConstraints::default()
            }),
            false,
        );
        assert_eq!(none_mapped.stored_categories(), [UNCLASSIFIED]);
        assert_eq!(none_mapped.stored_evidence(), ["Vitals"]);

        let ehr_only = m.classify(
            &AccessedContent::Query(QueryConstraints {
                resource_kinds: BTreeSet::from([ResourceKind::Ehr]),
                ..QueryConstraints::default()
            }),
            false,
        );
        assert_eq!(ehr_only.stored_categories(), [NONE]);
        assert_eq!(ehr_only.read_from, ReadFrom::ResourceKind);
    }

    #[test]
    fn the_mirror_rows_carry_tokens_and_none() {
        let classifier = map(MAP);
        let rows = classifier.rows();
        assert!(rows.contains(&("template", "admin note".to_owned(), NONE.to_owned())));
        assert!(rows.contains(&("template", "lab report".to_owned(), LAB.to_owned())));
        assert!(rows.contains(&(
            "archetype",
            "openehr-ehr-composition.imaging_report.v1".to_owned(),
            BEELD.to_owned()
        )));
    }
}
