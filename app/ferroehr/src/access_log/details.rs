// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log details: the facts of an access the draft's core
//! attributes do not hold, kept in the entry's `other_details` (#5127).
//!
//! The draft gives `ACCESS_LOG_ENTRY.other_details`, an archetypable
//! `ITEM_STRUCTURE` as on `FEEDER_AUDIT_DETAILS`, to the fields a
//! jurisdiction adds (`docs/specifications/access-log`, the package page). FerroEHR keeps
//! there what its projections to the IHE BALP `AuditEvent`, the DICOM audit
//! message and the EHDS access record carry beside the core: the audit forms
//! of the action, the outcome and the resource class, the subject the EHR
//! belongs to, the authentication facts, the category evidence and the
//! binding fields, so a projection reads the stored entry and nothing else.
//! The tree is one `ITEM_TREE` of `ELEMENT`s under the archetype id
//! [`ARCHETYPE`], each element named by its id-code ([`Detail`]), a
//! definition shipped and published like the access control archetypes
//! ([`crate::access_log::definition::DEFINITIONS`]), so another system reads the tree by its node ids. The
//! codes all have two digits, so their byte order, which orders the tree
//! here and in the audit schema, is their numeric order (#6143). No openEHR
//! spec governs these fields: our own design.

use openehr_base::prelude::ArchetypeId;
use openehr_rm::draft::access_control::terminology::Confidentiality;

use crate::access_log::definition::ARCHETYPE;
use openehr_rm::v1_2::common::archetyped::archetyped::Archetyped;
use openehr_rm::v1_2::data_structures::item_structure::item_structure::ItemStructure;
use openehr_rm::v1_2::data_structures::item_structure::item_tree::ItemTree;
use openehr_rm::v1_2::data_structures::representation::element::Element;
use openehr_rm::v1_2::data_structures::representation::item::Item;
use openehr_rm::v1_2::data_types::basic::data_value::DataValue;
use openehr_rm::v1_2::data_types::basic::dv_boolean::DvBoolean;
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::encapsulated::dv_parsable::DvParsable;
use openehr_rm::v1_2::data_types::quantity::dv_count::DvCount;
use openehr_rm::v1_2::data_types::text::dv_text::{DvText, DvTextData};

/// The formalism of a detail kept as JSON.
pub const JSON: &str = "application/json";

/// One detail of the tree, named by its id-code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    /// The DICOM `EventActionCode` (PS3.15 A.5.1): `C`, `R`, `U`, `D` or `E`.
    EventAction,
    /// The class of resource the access addressed, as the audit trail names
    /// it.
    ResourceClass,
    /// The DICOM `EventOutcomeIndicator` (PS3.15 A.5.1): 0, 4 or 8.
    OutcomeIndicator,
    /// The login the access was made under.
    Principal,
    /// The issuer of the caller's token.
    Issuer,
    /// The client the caller's token was issued to.
    ClientId,
    /// The `jti` of the caller's token.
    TokenId,
    /// A role the caller held, one element each.
    Role,
    /// The pseudonymisation domain, when it is not the resource class's own.
    Domain,
    /// The EHR the request named, when the core's `ehr_id` does not hold it.
    EhrId,
    /// The subject of care of the EHR, as `EHR_STATUS.subject` names it.
    Subject,
    /// Whether the subject of care could not be looked up.
    SubjectLookupFailed,
    /// The object the request named, when no item holds it.
    ObjectId,
    /// The purpose of use the caller declared.
    Purpose,
    /// The legal basis the deployment attributes to the access.
    LegalBasis,
    /// How many records the access served.
    ResultCount,
    /// An origin of the data, one element each, when no item holds them.
    Origin,
    /// How many distinct origins the data had.
    OriginCount,
    /// The digest of the category map the access was classified through.
    CategoryMapDigest,
    /// What the classification read, as JSON.
    CategoryContent,
    /// How the credential acted.
    ActingMode,
    /// The assurance level of the authentication.
    AssuranceLevel,
    /// The claim value the assurance level was read from.
    AssuranceValue,
    /// The identifier of the natural person who accessed.
    ProfessionalId,
    /// The purposes and roles the access token stated, as JSON.
    TokenAttributes,
    /// The party binding of the caller with every field, as JSON.
    Binding,
    /// The count and EHRs of a summary of refusals, as JSON.
    Summary,
    /// The DICOM `EventTypeCode` of an authentication record: `110122` Login
    /// or `110123` Logout.
    EventType,
    /// The level the access is recorded for review at.
    Review,
}

impl Detail {
    /// Every detail, in the order the tree holds them.
    pub const ALL: [Self; 29] = [
        Self::EventAction,
        Self::ResourceClass,
        Self::OutcomeIndicator,
        Self::Principal,
        Self::Issuer,
        Self::ClientId,
        Self::TokenId,
        Self::Role,
        Self::Domain,
        Self::EhrId,
        Self::Subject,
        Self::SubjectLookupFailed,
        Self::ObjectId,
        Self::Purpose,
        Self::LegalBasis,
        Self::ResultCount,
        Self::Origin,
        Self::OriginCount,
        Self::CategoryMapDigest,
        Self::CategoryContent,
        Self::ActingMode,
        Self::AssuranceLevel,
        Self::AssuranceValue,
        Self::ProfessionalId,
        Self::TokenAttributes,
        Self::Binding,
        Self::Summary,
        Self::EventType,
        Self::Review,
    ];

    /// The id-code of the detail's element.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::EventAction => "id11",
            Self::ResourceClass => "id12",
            Self::OutcomeIndicator => "id13",
            Self::Principal => "id14",
            Self::Issuer => "id15",
            Self::ClientId => "id16",
            Self::TokenId => "id17",
            Self::Role => "id19",
            Self::Domain => "id20",
            Self::EhrId => "id21",
            Self::Subject => "id22",
            Self::SubjectLookupFailed => "id23",
            Self::ObjectId => "id24",
            Self::Purpose => "id25",
            Self::LegalBasis => "id26",
            Self::ResultCount => "id27",
            Self::Origin => "id28",
            Self::OriginCount => "id29",
            Self::CategoryMapDigest => "id30",
            Self::CategoryContent => "id31",
            Self::ActingMode => "id32",
            Self::AssuranceLevel => "id33",
            Self::AssuranceValue => "id34",
            Self::ProfessionalId => "id35",
            Self::TokenAttributes => "id36",
            Self::Binding => "id37",
            Self::Summary => "id38",
            Self::EventType => "id39",
            Self::Review => "id40",
        }
    }

    /// The name the detail's element carries.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::EventAction => "event action",
            Self::ResourceClass => "resource class",
            Self::OutcomeIndicator => "outcome indicator",
            Self::Principal => "principal",
            Self::Issuer => "issuer",
            Self::ClientId => "client id",
            Self::TokenId => "token id",
            Self::Role => "role",
            Self::Domain => "domain",
            Self::EhrId => "ehr id",
            Self::Subject => "subject",
            Self::SubjectLookupFailed => "subject lookup failed",
            Self::ObjectId => "object id",
            Self::Purpose => "purpose",
            Self::LegalBasis => "legal basis",
            Self::ResultCount => "result count",
            Self::Origin => "origin",
            Self::OriginCount => "origin count",
            Self::CategoryMapDigest => "category map digest",
            Self::CategoryContent => "category content",
            Self::ActingMode => "acting mode",
            Self::AssuranceLevel => "assurance level",
            Self::AssuranceValue => "assurance value",
            Self::ProfessionalId => "professional id",
            Self::TokenAttributes => "token attributes",
            Self::Binding => "binding",
            Self::Summary => "summary",
            Self::EventType => "event type",
            Self::Review => "review",
        }
    }
}

/// The details tree being written: one element per value, ordered by the
/// codes of their details and, within one detail, in the order they are
/// added, so the tree's order follows from its content (#5703).
#[derive(Debug, Default)]
pub struct DetailsWriter {
    items: Vec<Item>,
}

impl DetailsWriter {
    /// Adds `value` as the element of `detail`.
    pub fn value(&mut self, detail: Detail, value: DataValue) {
        self.items.push(Item::Element(Box::new(Element {
            name: text(detail.name()),
            archetype_node_id: detail.code().to_owned(),
            uid: None,
            links: None,
            archetype_details: None,
            feeder_audit: None,
            null_flavour: None,
            value: Some(Box::new(value)),
            null_reason: None,
        })));
    }

    /// Adds `value` as a `DV_TEXT` element of `detail`, when there is one and
    /// it is not empty.
    pub fn text(&mut self, detail: Detail, value: Option<&str>) {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            self.value(detail, DataValue::DvText(text(value)));
        }
    }

    /// Adds each of `values` as a `DV_TEXT` element of `detail`.
    pub fn texts<'a>(&mut self, detail: Detail, values: impl IntoIterator<Item = &'a str>) {
        for value in values {
            self.value(detail, DataValue::DvText(text(value)));
        }
    }

    /// Adds `value` as a `DV_COUNT` element of `detail`, when there is one.
    pub fn count(&mut self, detail: Detail, value: Option<u64>) {
        if let Some(value) = value {
            self.value(
                detail,
                DataValue::DvCount(DvCount {
                    normal_status: None,
                    normal_range: None,
                    other_reference_ranges: None,
                    magnitude_status: None,
                    accuracy: None,
                    accuracy_is_percent: None,
                    magnitude: i64::try_from(value).unwrap_or(i64::MAX),
                }),
            );
        }
    }

    /// Adds a `DV_BOOLEAN` element of `detail` when `value` holds.
    pub fn flag(&mut self, detail: Detail, value: bool) {
        if value {
            self.value(detail, DataValue::DvBoolean(DvBoolean { value }));
        }
    }

    /// Adds `level` as a `DV_CODED_TEXT` element of `detail`, of
    /// `HL7_Confidentiality`, when there is one.
    pub fn level(&mut self, detail: Detail, level: Option<Confidentiality>) {
        if let Some(level) = level {
            self.value(
                detail,
                DataValue::DvText(DvText::DvCodedText(level.coded())),
            );
        }
    }

    /// Adds `value` as a `DV_IDENTIFIER` element of `detail`.
    pub fn identifier(&mut self, detail: Detail, value: DvIdentifier) {
        self.value(detail, DataValue::DvIdentifier(value));
    }

    /// Adds `json` as a `DV_PARSABLE` element of `detail` in [`JSON`], when
    /// there is one.
    pub fn json(&mut self, detail: Detail, json: Option<String>) {
        if let Some(value) = json {
            self.value(
                detail,
                DataValue::DvParsable(DvParsable {
                    charset: None,
                    language: None,
                    value,
                    formalism: JSON.to_owned(),
                }),
            );
        }
    }

    /// The tree of the details added; `None` when none was.
    #[must_use]
    pub fn finish(mut self) -> Option<ItemStructure> {
        if self.items.is_empty() {
            return None;
        }
        self.items
            .sort_by(|left, right| node(left).cmp(node(right)));
        Some(ItemStructure::ItemTree(Box::new(ItemTree {
            name: text("access details"),
            archetype_node_id: ARCHETYPE.to_owned(),
            uid: None,
            links: None,
            archetype_details: Some(Box::new(Archetyped {
                archetype_id: ArchetypeId {
                    value: ARCHETYPE.to_owned(),
                },
                template_id: None,
                rm_version: "1.2.0".to_owned(),
            })),
            feeder_audit: None,
            items: Some(self.items),
        })))
    }
}

/// The node id of the element or cluster `item`.
fn node(item: &Item) -> &str {
    match item {
        Item::Element(element) => &element.archetype_node_id,
        Item::Cluster(cluster) => &cluster.archetype_node_id,
    }
}

/// A plain `DV_TEXT` holding `value`.
fn text(value: &str) -> DvText {
    DvText::DvText(DvTextData {
        value: value.to_owned(),
        hyperlink: None,
        formatting: None,
        mappings: None,
        language: None,
        encoding: None,
    })
}

/// Why the details of an entry do not read.
#[derive(Debug, thiserror::Error)]
pub enum DetailsError {
    /// The entry's `other_details` is no access log details tree.
    #[error("the entry's other_details is not the access log details tree")]
    Foreign,
    /// A detail holds a value of another type than the one it is written as.
    #[error("the {0:?} detail holds a value of another type")]
    Type(Detail),
    /// A JSON detail does not read as the shape it is written in.
    #[error("the {detail:?} detail does not read")]
    Json {
        /// The detail.
        detail: Detail,
        /// Why it does not read.
        #[source]
        source: serde_json::Error,
    },
}

/// The details of one entry, read by detail.
#[derive(Debug, Default)]
pub struct Details<'a> {
    elements: Vec<&'a Element>,
}

impl<'a> Details<'a> {
    /// The details `other_details` holds; none when it holds none.
    ///
    /// # Errors
    /// [`DetailsError::Foreign`] when it holds a structure other than
    /// the access log details tree.
    pub fn of(other_details: Option<&'a ItemStructure>) -> Result<Self, DetailsError> {
        let Some(structure) = other_details else {
            return Ok(Self::default());
        };
        let ItemStructure::ItemTree(tree) = structure else {
            return Err(DetailsError::Foreign);
        };
        if tree.archetype_node_id != ARCHETYPE {
            return Err(DetailsError::Foreign);
        }
        let elements = tree
            .items
            .iter()
            .flatten()
            .map(|item| match item {
                Item::Element(element) => Ok(&**element),
                Item::Cluster(_) => Err(DetailsError::Foreign),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { elements })
    }

    /// The values of `detail`, in the order they were written.
    fn values(&self, detail: Detail) -> impl Iterator<Item = &'a DataValue> + '_ {
        self.elements
            .iter()
            .filter(move |element| element.archetype_node_id == detail.code())
            .filter_map(|element| element.value.as_deref())
    }

    /// The texts of `detail`.
    ///
    /// # Errors
    /// [`DetailsError::Type`] when a value is no `DV_TEXT`.
    pub fn texts(&self, detail: Detail) -> Result<Vec<String>, DetailsError> {
        self.values(detail)
            .map(|value| match value {
                DataValue::DvText(DvText::DvText(text)) => Ok(text.value.clone()),
                _ => Err(DetailsError::Type(detail)),
            })
            .collect()
    }

    /// The text of `detail`, when it holds one.
    ///
    /// # Errors
    /// [`DetailsError::Type`] when its value is no `DV_TEXT`.
    pub fn text(&self, detail: Detail) -> Result<Option<String>, DetailsError> {
        Ok(self.texts(detail)?.into_iter().next())
    }

    /// The count of `detail`, when it holds one.
    ///
    /// # Errors
    /// [`DetailsError::Type`] when its value is no non-negative `DV_COUNT`.
    pub fn count(&self, detail: Detail) -> Result<Option<u64>, DetailsError> {
        self.values(detail)
            .next()
            .map(|value| match value {
                DataValue::DvCount(count) => {
                    u64::try_from(count.magnitude).map_err(|_negative| DetailsError::Type(detail))
                }
                _ => Err(DetailsError::Type(detail)),
            })
            .transpose()
    }

    /// Whether `detail` is set.
    ///
    /// # Errors
    /// [`DetailsError::Type`] when its value is no `DV_BOOLEAN`.
    pub fn flag(&self, detail: Detail) -> Result<bool, DetailsError> {
        match self.values(detail).next() {
            None => Ok(false),
            Some(DataValue::DvBoolean(flag)) => Ok(flag.value),
            Some(_) => Err(DetailsError::Type(detail)),
        }
    }

    /// The `HL7_Confidentiality` level of `detail`, when it holds one.
    ///
    /// # Errors
    /// [`DetailsError::Type`] when its value is no level of that code system.
    pub fn level(&self, detail: Detail) -> Result<Option<Confidentiality>, DetailsError> {
        self.values(detail)
            .next()
            .map(|value| match value {
                DataValue::DvText(DvText::DvCodedText(coded)) => {
                    Confidentiality::read(coded).map_err(|_other| DetailsError::Type(detail))
                }
                _ => Err(DetailsError::Type(detail)),
            })
            .transpose()
    }

    /// The identifier of `detail`, when it holds one.
    ///
    /// # Errors
    /// [`DetailsError::Type`] when its value is no `DV_IDENTIFIER`.
    pub fn identifier(&self, detail: Detail) -> Result<Option<&'a DvIdentifier>, DetailsError> {
        self.values(detail)
            .next()
            .map(|value| match value {
                DataValue::DvIdentifier(identifier) => Ok(identifier),
                _ => Err(DetailsError::Type(detail)),
            })
            .transpose()
    }

    /// The JSON of `detail` read as a `T`, when it holds one.
    ///
    /// # Errors
    /// [`DetailsError::Type`] when its value is no `DV_PARSABLE` in [`JSON`],
    /// [`DetailsError::Json`] when the JSON does not read as a `T`.
    pub fn json<T: serde::de::DeserializeOwned>(
        &self,
        detail: Detail,
    ) -> Result<Option<T>, DetailsError> {
        self.values(detail)
            .next()
            .map(|value| match value {
                DataValue::DvParsable(parsable) if parsable.formalism == JSON => {
                    serde_json::from_str(&parsable.value)
                        .map_err(|source| DetailsError::Json { detail, source })
                }
                _ => Err(DetailsError::Type(detail)),
            })
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped definition is the archetype the tree is written under,
    /// and it constrains every detail at the node id and with the data type
    /// the writer gives it, so another system reads a stored tree by it.
    #[test]
    fn the_shipped_definition_describes_every_detail() {
        let [(file, text)] = crate::access_log::definition::DEFINITIONS;
        let id = file.strip_suffix(".adls").expect("an .adls file");
        assert_eq!(text.lines().nth(1).map(str::trim), Some(id));
        assert!(id.starts_with(&format!("{ARCHETYPE}.")), "{id}");
        assert!(text.contains("ITEM_TREE[id1]"));
        let typed = |detail: Detail| match detail {
            Detail::OutcomeIndicator | Detail::ResultCount | Detail::OriginCount => "DV_COUNT",
            Detail::Subject => "DV_IDENTIFIER",
            Detail::SubjectLookupFailed => "DV_BOOLEAN",
            Detail::Review => "DV_CODED_TEXT",
            Detail::CategoryContent
            | Detail::TokenAttributes
            | Detail::Binding
            | Detail::Summary => "DV_PARSABLE",
            _ => "DV_TEXT",
        };
        for detail in Detail::ALL {
            let code = detail.code();
            let element = format!("ELEMENT[{code}]");
            let at = text
                .find(&element)
                .unwrap_or_else(|| panic!("{detail:?} has no {element}"));
            let constraint = text
                .get(at..)
                .and_then(|rest| rest.lines().nth(1))
                .unwrap_or("");
            assert!(
                constraint.contains(typed(detail)),
                "{detail:?}: {constraint}"
            );
            assert!(
                text.contains(&format!("[\"{code}\"] = <text")),
                "{detail:?} has no term"
            );
        }
    }

    /// Every code has two digits, so the byte order the server and the audit
    /// schema order a tree by is the order of the details.
    #[test]
    fn the_byte_order_of_the_codes_is_the_order_of_the_details() {
        let codes = Detail::ALL.map(Detail::code);
        assert!(
            codes
                .iter()
                .all(|code| code.len() == 4 && code.starts_with("id"))
        );
        assert!(codes.windows(2).all(|pair| pair[0] < pair[1]), "{codes:?}");
    }

    /// A written tree names the shipped archetype, and a tree under another
    /// archetype is foreign.
    #[test]
    fn a_tree_is_written_and_read_under_the_shipped_archetype() {
        let mut writer = DetailsWriter::default();
        writer.text(Detail::Principal, Some("alice"));
        let tree = writer.finish().expect("a tree");
        let ItemStructure::ItemTree(written) = &tree else {
            panic!("an ITEM_TREE");
        };
        assert_eq!(written.archetype_node_id, ARCHETYPE);
        let details = Details::of(Some(&tree)).expect("the tree reads");
        assert_eq!(
            details.text(Detail::Principal).expect("a text"),
            Some("alice".to_owned())
        );
        let mut foreign = tree.clone();
        if let ItemStructure::ItemTree(tree) = &mut foreign {
            tree.archetype_node_id = "openEHR-EHR-ITEM_TREE.other.v1".to_owned();
        }
        assert!(matches!(
            Details::of(Some(&foreign)),
            Err(DetailsError::Foreign)
        ));
    }
}
