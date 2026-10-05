// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(clippy::expect_used, reason = "test assertions/diagnostics/fixtures")]
//! **Wire-level negatives for every `1..*` model container.**
//!
//! A container whose BMM cardinality has a lower bound of 1 emits as
//! `openehr_base::containers::NonEmptyVec<T>`, so the bound is carried by the
//! type: an empty list is unrepresentable in the model rather than merely
//! rejected. That is a TYPE-level proof, and on its own it says nothing about
//! the wire — it is the canonical-JSON and canonical-XML READERS that have to
//! refuse the empty spelling, and a reader that stopped doing so would still
//! compile.
//!
//! So each affected attribute gets an ASSERTED twin pair: a complete minimal
//! VALID instance that must decode, and the same instance with the mandatory
//! list emptied to `[]`, which must be REFUSED with the offending attribute
//! named. A silently loosened reader is then a failing build, not a quiet
//! drift.
//!
//! The nine attributes and the vendored class tables their `1..*` bound is read
//! from (all under `docs/specs/openehr/RM/docs/UML/classes/`):
//!
//! | attribute | class table |
//! |---|---|
//! | `CONTRIBUTION.versions` | `org.openehr.rm.common.contribution.adoc` §Attributes |
//! | `REVISION_HISTORY_ITEM.audits` | `org.openehr.rm.common.revision_history_item.adoc` §Attributes + §Invariants (`Audit_valid`: `not audits.is_empty`) |
//! | `REVISION_HISTORY.items` | `org.openehr.rm.common.revision_history.adoc` §Attributes |
//! | `CLUSTER.items` | `org.openehr.rm.data_structures.cluster.adoc` §Attributes |
//! | `DV_PARAGRAPH.items` | `org.openehr.rm.data_types.dv_paragraph.adoc` §Attributes + §Invariants (`Items_valid`: `not items.is_empty`) |
//! | `EXTRACT_MANIFEST.entities` | `org.openehr.rm.ehr_extract.extract_manifest.adoc` §Attributes |
//! | `ADDRESSED_MESSAGE.addressees` | `org.openehr.rm.ehr_extract.addressed_message.adoc` §Attributes |
//! | `PARTY.identities` | `org.openehr.rm.demographic.party.adoc` §Attributes + §Invariants (`Identities_valid`: `not identities.is_empty`), driven through the concrete `PERSON` subtype (`org.openehr.rm.demographic.person.adoc`) because `PARTY` is abstract and has no wire form of its own |
//! | `CONTACT.addresses` | `org.openehr.rm.demographic.contact.adoc` §Attributes |
//!
//! Both twins are authored as raw wire documents rather than built from the
//! typed model: the refusal twin is unrepresentable in the typed model by
//! construction (that is the whole point of the container type), so raw bytes
//! are the only way to author what the reader must reject — and authoring the
//! valid half the same way keeps the pair exactly one member apart.

use openehr_its::json::{JsonParseError, from_canonical_json};
use openehr_its::xml::from_canonical_xml;
use openehr_rm::prelude::{
    AddressedMessage, Cluster, Contact, Contribution, DvParagraph, ExtractManifest, Person,
    RevisionHistory, RevisionHistoryItem,
};
use serde_json::{Value, json};

/// One `1..*` attribute: its valid twin, and the reader that must decode it.
struct NonEmptyCase {
    /// `CLASS.attribute`, for the assertion messages.
    label: &'static str,
    /// The wire member name to empty for the refusal twin.
    attribute: &'static str,
    /// A complete minimal instance of the owning class.
    valid: Value,
    /// Decode the owning class from canonical-JSON text, discarding the value.
    decode: fn(&str) -> Result<(), JsonParseError>,
}

// ── shared sub-structures ───────────────────────────────────────────────────

/// An `AUDIT_DETAILS` instance carrying exactly its mandatory members
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.audit_details.adoc`
/// §Attributes: `system_id`, `time_committed`, `change_type`, `committer`).
fn audit_details() -> Value {
    json!({
        "_type": "AUDIT_DETAILS",
        "system_id": "ferroehr.local",
        "time_committed": { "_type": "DV_DATE_TIME", "value": "2026-07-29T10:11:12Z" },
        "change_type": {
            "_type": "DV_CODED_TEXT",
            "value": "creation",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "249"
            }
        },
        "committer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" }
    })
}

/// A minimal `ITEM_TREE` carrying one `ELEMENT` — the `ITEM_STRUCTURE` payload
/// the demographic `details` slots take
/// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_structures.item_tree.adoc`
/// §Attributes).
fn item_tree(node_id: &str) -> Value {
    json!({
        "_type": "ITEM_TREE",
        "name": { "_type": "DV_TEXT", "value": "tree" },
        "archetype_node_id": node_id,
        "items": [{
            "_type": "ELEMENT",
            "name": { "_type": "DV_TEXT", "value": "family name" },
            "archetype_node_id": "at0003",
            "value": { "_type": "DV_TEXT", "value": "Smith" }
        }]
    })
}

// ── the valid twins, one per case ───────────────────────────────────────────

/// A `CONTRIBUTION` over one version reference
/// (`…org.openehr.rm.common.contribution.adoc` §Attributes: `uid`, `versions`,
/// `audit`).
fn contribution() -> Value {
    json!({
        "_type": "CONTRIBUTION",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000001" },
        "versions": [{
            "_type": "OBJECT_REF",
            "namespace": "local",
            "type": "VERSIONED_COMPOSITION",
            "id": { "_type": "HIER_OBJECT_ID", "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000002" }
        }],
        "audit": audit_details()
    })
}

/// A `REVISION_HISTORY_ITEM` over one audit
/// (`…org.openehr.rm.common.revision_history_item.adoc` §Attributes:
/// `version_id`, `audits`).
fn revision_history_item() -> Value {
    json!({
        "_type": "REVISION_HISTORY_ITEM",
        "version_id": {
            "_type": "OBJECT_VERSION_ID",
            "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000001::ferroehr.local::1"
        },
        "audits": [audit_details()]
    })
}

/// A `REVISION_HISTORY` over one item
/// (`…org.openehr.rm.common.revision_history.adoc` §Attributes: `items`).
fn revision_history() -> Value {
    json!({ "_type": "REVISION_HISTORY", "items": [revision_history_item()] })
}

/// A `CLUSTER` over one `ELEMENT` (`…org.openehr.rm.data_structures.cluster.adoc`
/// §Attributes: `items`, plus the inherited `LOCATABLE` mandatories `name` and
/// `archetype_node_id`).
fn cluster() -> Value {
    json!({
        "_type": "CLUSTER",
        "name": { "_type": "DV_TEXT", "value": "blood pressure" },
        "archetype_node_id": "at0001",
        "items": [{
            "_type": "ELEMENT",
            "name": { "_type": "DV_TEXT", "value": "systolic" },
            "archetype_node_id": "at0004",
            "value": { "_type": "DV_TEXT", "value": "120" }
        }]
    })
}

/// A `DV_PARAGRAPH` over one `DV_TEXT`
/// (`…org.openehr.rm.data_types.dv_paragraph.adoc` §Attributes: `items`).
fn dv_paragraph() -> Value {
    json!({
        "_type": "DV_PARAGRAPH",
        "items": [{ "_type": "DV_TEXT", "value": "first line" }]
    })
}

/// An `EXTRACT_MANIFEST` over one entity manifest
/// (`…org.openehr.rm.ehr_extract.extract_manifest.adoc` §Attributes:
/// `entities`; `EXTRACT_ENTITY_MANIFEST`'s only mandatory member is
/// `extract_id_key`).
fn extract_manifest() -> Value {
    json!({
        "_type": "EXTRACT_MANIFEST",
        "entities": [{ "_type": "EXTRACT_ENTITY_MANIFEST", "extract_id_key": "subject-1" }]
    })
}

/// An `ADDRESSED_MESSAGE` to one addressee
/// (`…org.openehr.rm.ehr_extract.addressed_message.adoc` §Attributes: `sender`,
/// `sender_reference`, `addressees`, `message`).
fn addressed_message() -> Value {
    json!({
        "_type": "ADDRESSED_MESSAGE",
        "sender": "sending.example.org",
        "sender_reference": "message-0001",
        "addressees": ["receiving.example.org"],
        "message": {
            "_type": "MESSAGE",
            "audit": audit_details(),
            "author": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" },
            "content": {
                "_type": "SYNC_EXTRACT_REQUEST",
                "specification": { "_type": "SYNC_EXTRACT_SPEC", "includes_versions": false }
            }
        }
    })
}

/// A `PERSON` with one identity — the concrete `PARTY` subtype the abstract
/// `identities` bound is exercised through. `uid` and `archetype_details` are
/// present because `…org.openehr.rm.demographic.party.adoc` §Invariants states
/// `Uid_mandatory` and `Is_archetype_root`, so a genuinely valid `PARTY` twin
/// carries both.
fn person() -> Value {
    json!({
        "_type": "PERSON",
        "name": { "_type": "DV_TEXT", "value": "person" },
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000003" },
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {
                "_type": "ARCHETYPE_ID",
                "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1"
            },
            "rm_version": "1.2.0"
        },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "archetype_node_id": "at0001",
            "details": item_tree("at0002")
        }]
    })
}

/// A `CONTACT` with one address (`…org.openehr.rm.demographic.contact.adoc`
/// §Attributes: `addresses`).
fn contact() -> Value {
    json!({
        "_type": "CONTACT",
        "name": { "_type": "DV_TEXT", "value": "mail" },
        "archetype_node_id": "at0001",
        "addresses": [{
            "_type": "ADDRESS",
            "name": { "_type": "DV_TEXT", "value": "postal" },
            "archetype_node_id": "at0002",
            "details": item_tree("at0004")
        }]
    })
}

/// The nine `1..*` attributes the RM declares, each with its valid twin and the
/// concrete reader entry point that must decode it.
fn cases() -> Vec<NonEmptyCase> {
    vec![
        NonEmptyCase {
            label: "CONTRIBUTION.versions",
            attribute: "versions",
            valid: contribution(),
            decode: |text: &str| from_canonical_json::<Contribution>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "REVISION_HISTORY_ITEM.audits",
            attribute: "audits",
            valid: revision_history_item(),
            decode: |text: &str| from_canonical_json::<RevisionHistoryItem>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "REVISION_HISTORY.items",
            attribute: "items",
            valid: revision_history(),
            decode: |text: &str| from_canonical_json::<RevisionHistory>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "CLUSTER.items",
            attribute: "items",
            valid: cluster(),
            decode: |text: &str| from_canonical_json::<Cluster>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "DV_PARAGRAPH.items",
            attribute: "items",
            valid: dv_paragraph(),
            decode: |text: &str| from_canonical_json::<DvParagraph>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "EXTRACT_MANIFEST.entities",
            attribute: "entities",
            valid: extract_manifest(),
            decode: |text: &str| from_canonical_json::<ExtractManifest>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "ADDRESSED_MESSAGE.addressees",
            attribute: "addressees",
            valid: addressed_message(),
            decode: |text: &str| from_canonical_json::<AddressedMessage>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "PARTY.identities (via PERSON)",
            attribute: "identities",
            valid: person(),
            decode: |text: &str| from_canonical_json::<Person>(text).map(|_| ()),
        },
        NonEmptyCase {
            label: "CONTACT.addresses",
            attribute: "addresses",
            valid: contact(),
            decode: |text: &str| from_canonical_json::<Contact>(text).map(|_| ()),
        },
    ]
}

/// Serialize a fixture to canonical-JSON text — the same door the wire uses.
fn wire_text(document: &Value) -> String {
    serde_json::to_string(document).expect("a fixture value serializes")
}

/// The refusal twin: `valid` with `attribute` replaced by `[]`.
///
/// Also asserts the valid twin actually carried a NON-EMPTY array there, so a
/// fixture whose member name drifted (or whose list was already empty) fails
/// here instead of quietly making the refusal assertion vacuous.
fn emptied(valid: &Value, attribute: &str, label: &str) -> Value {
    let mut document = valid.clone();
    let object = document
        .as_object_mut()
        .expect("every fixture root is a JSON object");
    let previous = object.insert(attribute.to_owned(), Value::Array(Vec::new()));
    assert!(
        previous
            .as_ref()
            .and_then(Value::as_array)
            .is_some_and(|members| !members.is_empty()),
        "{label}: the valid twin must carry a non-empty `{attribute}` to empty, got {previous:?}"
    );
    document
}

/// Exactly the nine attributes the RM declares `1..*` are covered — a dropped
/// or renamed one fails here rather than narrowing the battery silently.
#[test]
fn the_table_covers_every_declared_nonempty_container() {
    let labels: Vec<&str> = cases().iter().map(|case| case.label).collect();
    assert_eq!(
        labels,
        vec![
            "CONTRIBUTION.versions",
            "REVISION_HISTORY_ITEM.audits",
            "REVISION_HISTORY.items",
            "CLUSTER.items",
            "DV_PARAGRAPH.items",
            "EXTRACT_MANIFEST.entities",
            "ADDRESSED_MESSAGE.addressees",
            "PARTY.identities (via PERSON)",
            "CONTACT.addresses",
        ],
        "the nine RM `1..*` containers must each keep their twin pair"
    );
}

/// **Valid twin.** Every fixture is a complete instance of its class and reads
/// through the strict canonical-JSON reader — so the refusal twin below differs
/// from a decodable document in exactly one respect: the emptied list.
#[test]
fn a_populated_mandatory_list_reads() {
    for case in cases() {
        let text = wire_text(&case.valid);
        let outcome = (case.decode)(&text);
        assert!(
            outcome.is_ok(),
            "{}: the valid twin must read, got {outcome:?}",
            case.label
        );
    }
}

/// **Refusal twin.** The same document with the mandatory list spelled `[]` is
/// refused at PARSE, and the refusal names the offending attribute's path.
#[test]
fn an_empty_mandatory_list_is_refused_with_its_path() {
    for case in cases() {
        let document = emptied(&case.valid, case.attribute, case.label);
        let text = wire_text(&document);
        let error = (case.decode)(&text)
            .expect_err("an empty `1..*` list must be refused at the wire boundary");
        let rendered = error.to_string();
        assert!(
            rendered.contains("at least one member"),
            "{}: the refusal must state the cardinality bound, got: {rendered}",
            case.label
        );
        assert_eq!(
            error.path(),
            [format!(".{}", case.attribute)],
            "{}: the refusal must name the offending attribute's path, got: {rendered}",
            case.label
        );
    }
}

// ── the canonical-XML half: one representative attribute, both twins ─────────

/// The canonical-XML valid twin of the `CLUSTER.items` pair.
///
/// Canonical XML has no `[]` spelling: a repeated element with zero occurrences
/// IS absence, so the refusal twin ([`CLUSTER_XML_EMPTY`]) is this document with
/// its `<items>` element removed. That is precisely why the check has to exist
/// on this side too — the JSON reader's `[]` door does not exist here, and the
/// bound is enforced only where the container is constructed.
const CLUSTER_XML_VALID: &str = r#"<cluster xmlns="http://schemas.openehr.org/v1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" archetype_node_id="at0001">
  <name><value>blood pressure</value></name>
  <items xsi:type="ELEMENT" archetype_node_id="at0004">
    <name><value>systolic</value></name>
    <value xsi:type="DV_TEXT"><value>120</value></value>
  </items>
</cluster>"#;

/// The canonical-XML refusal twin: the same `<cluster>` with no `<items>` child
/// at all.
const CLUSTER_XML_EMPTY: &str = r#"<cluster xmlns="http://schemas.openehr.org/v1" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" archetype_node_id="at0001">
  <name><value>blood pressure</value></name>
</cluster>"#;

/// **Valid twin, canonical XML.** A `<cluster>` with one `<items>` child reads.
#[test]
fn a_populated_cluster_items_reads_from_canonical_xml() {
    let cluster: Cluster =
        from_canonical_xml(CLUSTER_XML_VALID).expect("the valid XML twin must read");
    assert_eq!(
        cluster.items.len(),
        1,
        "the one authored `<items>` child must survive the read"
    );
}

/// **Refusal twin, canonical XML.** A `<cluster>` with no `<items>` child is
/// refused, naming the element.
#[test]
fn a_cluster_without_items_is_refused_by_the_canonical_xml_reader() {
    let error = from_canonical_xml::<Cluster>(CLUSTER_XML_EMPTY)
        .expect_err("a CLUSTER with no items must be refused at the wire boundary");
    let rendered = error.to_string();
    assert!(
        rendered.contains("items"),
        "the refusal must name the offending element, got: {rendered}"
    );
    assert!(
        rendered.contains("at least one member"),
        "the refusal must state the cardinality bound, got: {rendered}"
    );
}

/// The #1730 optional-list twin (`dv_text.adoc` §Invariants `Mappings_valid`):
/// a present-but-empty 0..1 list refuses at parse; absent passes. Pins the
/// corpus adjudication of `flat_folder_insert.json` (`common::excluded`).
#[test]
fn a_present_but_empty_optional_nonempty_list_refuses_at_parse() {
    let refused = from_canonical_json::<openehr_rm::v1_2::data_types::text::dv_text::DvTextData>(
        r#"{"_type":"DV_TEXT","value":"x","mappings":[]}"#,
    )
    .expect_err("Mappings_valid holds by construction (#1730)");
    assert!(refused.to_string().contains("mappings"), "{refused}");
    from_canonical_json::<openehr_rm::v1_2::data_types::text::dv_text::DvTextData>(
        r#"{"_type":"DV_TEXT","value":"x"}"#,
    )
    .expect("absent mappings is legal (0..1)");
}
