// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test assertions/diagnostics/fixtures"
)]
//! XSD validity and round trips of the demographic documents the server writes
//! as canonical XML.
//!
//! The demographic API serves PERSON, ORGANISATION, GROUP, AGENT and ROLE in
//! both profiles (the stable one through `openehr_rm::v1_1`, whose `PARTY`
//! still carries `reverse_relationships`) and `PARTY_RELATIONSHIP`, each under
//! the published `items` root with `xsi:type`. ITS-REST
//! `specifications/docs/overview/Resources.md` §"XML Format" requires those
//! documents to conform to the published XSDs.
//!
//! ## The schemas each profile is judged by
//!
//! - **nsv2:** `its-xml-2.0.0-nsv2/RM/latest/Demographic.xsd` for the
//!   development profile and `RM/Release-1.1.0/Demographic.xsd` for the stable
//!   one, each with the `BASE/latest/BaseTypes.xsd` its `DataTypes.xsd`
//!   includes. ITS-XML publishes no RM 1.2.0 folder; `RM/latest/README.adoc`
//!   names its schemas "openEHR Release-1.1.0", and the two folders are byte
//!   identical.
//! - **nsv1:** none. The released v1 lineage, upstream tags `Release-1.0.2`
//!   (`780d522e`) and `Release-1.0.2v2` (`f7a93777`), publishes no
//!   `Demographic.xsd`: a v1 `Demographic.xsd` existed only on untagged
//!   development commits before upstream `1dead143` (2019-01-31) moved every
//!   RM schema to the v2 namespace. A v1 document is the v2 document under
//!   another root `xmlns`, which `the_nsv1_document_differs_only_by_the_namespace`
//!   pins, so the nsv2 verdict is the only schema verdict a v1 demographic
//!   document can have.
//!
//! ## The nsv2 `archetypeNodeId` facet
//!
//! The verbatim nsv2 bundle cannot be compiled (the pinned
//! `nsv2_lineage_is_uncompilable_by_a_conformant_xsd_processor`): its
//! `archetypeNodeId` facet is a Perl expression (its comment links regex101),
//! with `(?:…)` groups XML Schema Part 2 Appendix F does not define and `\w`,
//! which Appendix F defines without `_` (§F.1.1, `\w` excludes `\p{P}`).
//! This gate copies the schema files into a scratch tree and rewrites that one
//! facet for XSD 1.0, `(?:` to `(`, `\w` to `[A-Za-z0-9_]` and `\d` to `[0-9]`,
//! and asserts the result is the facet upstream's XSD 1.0 rewrite carries
//! (`specifications-ITS-XML` commit `1edd3ad`, SPECPR-492, on master and in no
//! release yet, #5034). The vendored files are never modified.

use openehr_its::json::{from_canonical_value, to_canonical_json};
use openehr_its::xml::LOCATABLE_ROOT;
use openehr_its::xml::runtime::{FromXml, Namespace, ToXml, from_xml, to_xml_declared};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use crate::xml_xsd_validity::{Outcome, schemas_root, scratch, xmllint};

/// The nsv2 target namespace.
const NSV2: &str = "http://schemas.openehr.org/v2";

/// The `xs:simpleType` whose pattern facet is rewritten for XSD 1.0.
const FACET_OPEN: &str = "<xs:simpleType name=\"archetypeNodeId\">";

// ---------------------------------------------------------------------------
// The derived nsv2 tree and its driver schema
// ---------------------------------------------------------------------------

/// Copies the `*.xsd` files of `from` into `to`, rewriting `BaseTypes.xsd`'s
/// `archetypeNodeId` facet through [`xsd10_facet`].
fn copy_schemas(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("create derived schema dir");
    let mut names: Vec<PathBuf> = std::fs::read_dir(from)
        .expect("read vendored schema dir")
        .map(|e| e.expect("dir entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "xsd"))
        .collect();
    names.sort();
    for path in names {
        let name = path.file_name().expect("file name");
        let text = std::fs::read_to_string(&path).expect("read vendored XSD");
        let text = if name == "BaseTypes.xsd" {
            xsd10_facet(&text)
        } else {
            assert!(
                !text.contains("(?:"),
                "{}: a Perl group outside BaseTypes.xsd needs its own adjudication",
                path.display()
            );
            text
        };
        std::fs::write(to.join(name), text).expect("write derived XSD");
    }
}

/// The `archetypeNodeId` pattern of upstream's XSD 1.0 rewrite
/// (`specifications-ITS-XML` `1edd3ad`, `components/BASE/latest/BaseTypes.xsd`).
const XSD10_PATTERN: &str = "((([A-Za-z0-9_]+(\\.[A-Za-z0-9_-]+)+::)?([a-zA-Z][A-Za-z0-9_]+)-\
     ([a-zA-Z][A-Za-z0-9_]+)-([a-zA-Z][A-Za-z0-9_]+)\\.([a-zA-Z][A-Za-z0-9_]+(-[A-Za-z0-9_]+)?)\\.\
     v[0-9]+(\\.[0-9]+\\.[0-9]+)?(-(rc|alpha)(\\.[0-9]+)?)?)|((at|id)[0-9]+(\\.[0-9]+)*))";

/// Rewrites the Perl `archetypeNodeId` facet for XSD 1.0, refusing a schema
/// that carries a Perl group anywhere else or a facet the rewrite does not
/// turn into [`XSD10_PATTERN`].
fn xsd10_facet(text: &str) -> String {
    let start = text
        .find(FACET_OPEN)
        .expect("the archetypeNodeId simpleType");
    let end = start
        + text
            .get(start..)
            .and_then(|t| t.find("</xs:simpleType>"))
            .expect("the end of the archetypeNodeId simpleType");
    let (head, rest) = text.split_at(start);
    let (facet, tail) = rest.split_at(end - start);
    assert!(
        !head.contains("(?:") && !tail.contains("(?:"),
        "BaseTypes.xsd carries a Perl group outside the archetypeNodeId facet"
    );
    let rewritten = facet
        .replace("(?:", "(")
        .replace("[\\w-]", "[A-Za-z0-9_-]")
        .replace("\\w", "[A-Za-z0-9_]")
        .replace("\\d", "[0-9]");
    assert!(
        rewritten.contains(&format!("value=\"{XSD10_PATTERN}\"")),
        "the vendored archetypeNodeId facet no longer rewrites to upstream's XSD 1.0 facet: \
         re-adjudicate this gate against it (#5034)\n{rewritten}"
    );
    format!("{head}{rewritten}{tail}")
}

/// Builds the derived nsv2 tree for RM folder `rm` under the scratch name
/// `tag` (one per test, so no two tests write the same file) and writes a
/// driver schema that includes its `Demographic.xsd` and declares the
/// published `items` root.
fn demographic_driver(rm: &str, tag: &str) -> PathBuf {
    let vendored = schemas_root().join("its-xml-2.0.0-nsv2");
    let derived = scratch().join(format!("nsv2-xsd10-{tag}"));
    copy_schemas(&vendored.join("BASE/latest"), &derived.join("BASE/latest"));
    copy_schemas(&vendored.join("RM").join(rm), &derived.join("RM").join(rm));
    let include = derived.join("RM").join(rm).join("Demographic.xsd");
    let xsd = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns=\"{NSV2}\" \
         targetNamespace=\"{NSV2}\" elementFormDefault=\"qualified\">\n  \
         <xs:include schemaLocation=\"file://{}\"/>\n  \
         <xs:element name=\"{LOCATABLE_ROOT}\" type=\"LOCATABLE\"/>\n\
         </xs:schema>\n",
        include.display()
    );
    let path = scratch().join(format!("demographic_{tag}.xsd"));
    std::fs::write(&path, xsd).expect("write driver schema");
    path
}

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

/// A `DV_TEXT` name.
fn text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// An `ITEM_TREE` holding one text `ELEMENT`.
fn tree(node: &str, value: &str) -> Value {
    json!({
        "_type": "ITEM_TREE", "name": text("tree"), "archetype_node_id": node,
        "items": [{ "_type": "ELEMENT", "name": text("element"),
            "archetype_node_id": "at0100", "value": text(value) }]
    })
}

/// A `DV_INTERVAL<DV_DATE>` from `lower` with no upper bound.
fn validity(lower: &str) -> Value {
    json!({
        "_type": "DV_INTERVAL",
        "lower": { "_type": "DV_DATE", "value": lower },
        "lower_unbounded": false, "upper_unbounded": true,
        "lower_included": true, "upper_included": false
    })
}

/// A `PARTY_REF` to the versioned party `id` of class `kind`.
fn party_ref(kind: &str, id: &str) -> Value {
    json!({
        "_type": "PARTY_REF", "namespace": "demographic", "type": kind,
        "id": { "_type": "HIER_OBJECT_ID", "value": id }
    })
}

/// The id every vector's party is addressed by.
const SELF_ID: &str = "0e1b2c3d-0000-4000-8000-000000000001";

/// The party every relationship points at.
const OTHER_ID: &str = "0e1b2c3d-0000-4000-8000-000000000002";

/// A `PARTY_RELATIONSHIP` with every optional member set.
fn relationship(source_kind: &str) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "name": text("employment"),
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "details": tree("at0200", "since hire"),
        "time_validity": validity("2020-01-01"),
        "source": party_ref(source_kind, SELF_ID),
        "target": party_ref("ORGANISATION", OTHER_ID)
    })
}

/// The PARTY members every subtype shares: identities, contacts with
/// addresses and `time_validity`, relationships and details.
fn party(kind: &str) -> Value {
    json!({
        "_type": kind,
        "name": text(&format!("{kind} vector")),
        "archetype_node_id": format!("openEHR-DEMOGRAPHIC-{kind}.generic.v1"),
        "uid": { "_type": "HIER_OBJECT_ID", "value": SELF_ID },
        "identities": [{
            "_type": "PARTY_IDENTITY", "name": text("legal identity"),
            "archetype_node_id": "at0001", "details": tree("at0002", "legal name")
        }],
        "contacts": [{
            "_type": "CONTACT", "name": text("daytime"), "archetype_node_id": "at0010",
            "time_validity": validity("2021-02-03"),
            "addresses": [
                { "_type": "ADDRESS", "name": text("phone"), "archetype_node_id": "at0011",
                  "details": tree("at0012", "+31 10 000 0000") },
                { "_type": "ADDRESS", "name": text("email"), "archetype_node_id": "at0013",
                  "details": tree("at0014", "person@example.org") }
            ]
        }],
        "relationships": [relationship(kind)],
        "details": tree("at0020", "details")
    })
}

/// An ACTOR subtype with `roles` and `languages` beside the PARTY members.
fn actor(kind: &str) -> Value {
    let mut value = party(kind);
    value["roles"] = json!([party_ref("ROLE", "0e1b2c3d-0000-4000-8000-000000000003")]);
    value["languages"] = json!([text("en"), text("nl")]);
    value
}

/// A ROLE with capabilities (each with credentials and `time_validity`), its
/// own `time_validity` and its performer.
fn role() -> Value {
    let mut value = party("ROLE");
    value["capabilities"] = json!([{
        "_type": "CAPABILITY", "name": text("prescribing"), "archetype_node_id": "at0030",
        "credentials": tree("at0031", "licence 42"),
        "time_validity": validity("2022-03-04")
    }]);
    value["time_validity"] = validity("2022-01-01");
    value["performer"] = party_ref("PERSON", OTHER_ID);
    value
}

/// A released (RM 1.1.0) party: `value` with a `reverse_relationships` entry.
fn released(mut value: Value) -> Value {
    value["reverse_relationships"] = json!([{
        "_type": "LOCATABLE_REF", "namespace": "local", "type": "PARTY_RELATIONSHIP",
        "id": { "_type": "HIER_OBJECT_ID", "value": OTHER_ID },
        "path": "/relationships[openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1]"
    }]);
    value
}

// ---------------------------------------------------------------------------
// The vectors
// ---------------------------------------------------------------------------

/// The canonical XML of `value` re-typed as `T`, in namespace `ns`, under the
/// root the demographic API serves.
fn serve<T: DeserializeOwned + ToXml>(value: &Value, ns: Namespace) -> String {
    let typed: T =
        from_canonical_value(value).unwrap_or_else(|e| panic!("a typed {}: {e}", value["_type"]));
    to_xml_declared(&typed, LOCATABLE_ROOT, "LOCATABLE", ns).expect("serialize canonical XML")
}

/// Proves JSON → XML → JSON is byte-stable and XML → RM → XML reproduces the
/// served bytes, returning the served nsv2 document.
fn round_trip<T>(value: &Value) -> String
where
    T: DeserializeOwned + Serialize + ToXml + FromXml,
{
    let label = value["_type"].as_str().expect("a _type");
    let typed: T = from_canonical_value(value).expect("typed value");
    let json = to_canonical_json(&typed);
    let xml = serve::<T>(value, Namespace::V2);
    let back: T = from_xml(&xml).unwrap_or_else(|e| panic!("{label}: parse back: {e}\n{xml}"));
    assert_eq!(
        to_canonical_json(&back),
        json,
        "{label}: JSON -> XML -> JSON must be byte-stable"
    );
    let again = to_xml_declared(&back, LOCATABLE_ROOT, "LOCATABLE", Namespace::V2)
        .expect("serialize again");
    assert_eq!(again, xml, "{label}: XML -> RM -> XML must be byte-stable");
    xml
}

/// Validates `xml` against the driver schema `driver`, failing with the
/// `xmllint` diagnostics.
fn assert_valid(driver: &Path, label: &str, xml: &str) {
    let document = scratch().join(format!("demographic.{label}.xml"));
    std::fs::write(&document, xml).expect("write document");
    match xmllint(driver, &document) {
        Outcome::Valid => {}
        other => panic!("{label}: the served document is not valid nsv2 XML: {other:?}\n{xml}"),
    }
}

/// The development profile (`openehr_rm::v1_2`) against `RM/latest`.
#[test]
fn development_documents_validate_against_the_nsv2_demographic_schema() {
    use openehr_rm::v1_2::demographic::{
        agent::Agent, group::Group, organisation::Organisation,
        party_relationship::PartyRelationship, person::Person, role::Role,
    };
    let driver = demographic_driver("latest", "dev");
    let documents = [
        ("dev.person", round_trip::<Person>(&actor("PERSON"))),
        (
            "dev.organisation",
            round_trip::<Organisation>(&actor("ORGANISATION")),
        ),
        ("dev.group", round_trip::<Group>(&actor("GROUP"))),
        ("dev.agent", round_trip::<Agent>(&actor("AGENT"))),
        ("dev.role", round_trip::<Role>(&role())),
        (
            "dev.party_relationship",
            round_trip::<PartyRelationship>(&relationship("PERSON")),
        ),
    ];
    for (label, xml) in documents {
        assert_valid(&driver, label, &xml);
    }
}

/// The stable profile (`openehr_rm::v1_1`) against `RM/Release-1.1.0`, every
/// party carrying `reverse_relationships`.
#[test]
fn stable_documents_validate_against_the_release_1_1_0_demographic_schema() {
    use openehr_rm::v1_1::demographic::{
        agent::Agent, group::Group, organisation::Organisation, person::Person, role::Role,
    };
    let driver = demographic_driver("Release-1.1.0", "stable");
    let documents = [
        (
            "stable.person",
            round_trip::<Person>(&released(actor("PERSON"))),
        ),
        (
            "stable.organisation",
            round_trip::<Organisation>(&released(actor("ORGANISATION"))),
        ),
        (
            "stable.group",
            round_trip::<Group>(&released(actor("GROUP"))),
        ),
        (
            "stable.agent",
            round_trip::<Agent>(&released(actor("AGENT"))),
        ),
        ("stable.role", round_trip::<Role>(&released(role()))),
    ];
    for (label, xml) in documents {
        assert!(
            xml.contains("<reverse_relationships"),
            "{label}: the released party must carry reverse_relationships"
        );
        assert_valid(&driver, label, &xml);
    }
}

/// The derived facet still refuses what the upstream expression refuses, so
/// the rewrite cannot have widened it to accept any node id.
#[test]
fn the_rewritten_facet_still_judges_archetype_node_ids() {
    use openehr_rm::v1_2::demographic::person::Person;
    let driver = demographic_driver("latest", "facet");
    let mut value = actor("PERSON");
    value["identities"][0]["archetype_node_id"] = json!("not a node id");
    let document = scratch().join("demographic.bad_node_id.xml");
    std::fs::write(&document, serve::<Person>(&value, Namespace::V2)).expect("write document");
    match xmllint(&driver, &document) {
        Outcome::Invalid(diagnostics) => assert!(
            diagnostics.contains("'not a node id' is not accepted by the pattern"),
            "rejected for another reason than the facet: {diagnostics}"
        ),
        other => panic!("the rewritten facet accepted a malformed node id: {other:?}"),
    }
}

/// The nsv1 bundle has no `Demographic.xsd`, and a v1 demographic document is
/// the nsv2 one under the v1 root `xmlns`, byte for byte otherwise.
#[test]
fn the_nsv1_document_differs_only_by_the_namespace() {
    use openehr_rm::v1_2::demographic::{person::Person, role::Role};
    let nsv1 = schemas_root().join("its-xml-1.0.2-nsv1");
    let mut files: Vec<String> = Vec::new();
    for dir in ["ALL", "AOM2"] {
        for entry in std::fs::read_dir(nsv1.join(dir)).expect("read nsv1 dir") {
            let name = entry.expect("dir entry").file_name();
            files.push(name.to_string_lossy().into_owned());
        }
    }
    assert!(
        !files.iter().any(|f| f == "Demographic.xsd"),
        "the nsv1 bundle now carries a Demographic.xsd: validate v1 documents against it"
    );
    for (label, v1, v2) in [
        (
            "person",
            serve::<Person>(&actor("PERSON"), Namespace::V1),
            serve::<Person>(&actor("PERSON"), Namespace::V2),
        ),
        (
            "role",
            serve::<Role>(&role(), Namespace::V1),
            serve::<Role>(&role(), Namespace::V2),
        ),
    ] {
        assert_eq!(
            v1.replace("http://schemas.openehr.org/v1", NSV2),
            v2,
            "{label}: the two lineages must differ only by the root xmlns"
        );
    }
}
