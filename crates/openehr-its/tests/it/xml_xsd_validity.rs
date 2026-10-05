// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::panic,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "test assertions/diagnostics/fixtures"
)]
//! XSD-validity gate: does a served canonical-XML document conform to the
//! schema its own namespace declares?
//!
//! ITS-REST `specifications/docs/overview/Resources.md` §"XML Format" is
//! unconditional — "When resources are serialized in **canonical XML** format,
//! both request payloads and responses MUST conform to the [published XSDs]",
//! whose link target is `https://specifications.openehr.org/releases/ITS-XML/latest`.
//! Every other XML gate in this crate proves codec self-consistency
//! (`xml_roundtrip`), parity with a foreign serializer (`xml_c14n`,
//! `xml_ehrbase`) or namespace selection (`xml_namespace`); none of them ever
//! asked an XSD processor whether the bytes we serve are valid. This gate does,
//! against BOTH vendored lineages, and pins every divergence with its
//! adjudication — so a NEW divergence fails the build while the known set stays
//! honest.
//!
//! ## Method
//!
//! `xmllint --schema` (the same tool the C14N gate shells to) over a **driver
//! schema written at test time**: it `xs:include`s the vendored bundle files
//! verbatim by absolute path and adds only the global `xs:element` declarations
//! the bundle itself lacks. The vendored schemas are never modified, and the
//! driver adds no type, facet or content model — only a document root to hang
//! validation on, which is exactly what ITS-XML withholds for most REST
//! resources.
//! The divergence sweeps below are computed from the generated RM model
//! (`openehr_rm::v1_2::model`) against the XSD text, so they re-derive themselves on
//! every run instead of trusting a hand-maintained attribute list.
//!
//! ## What the sweep establishes
//!
//! The standing premise that the two lineages "differ only by the root `xmlns`"
//! is true of our SERIALIZER and false of the SCHEMAS. `docs/specs/openehr/
//! ITS-XML/README.adoc` §"Releases and IM Versions" describes the 2.0.0 change
//! to the *schemas* ("the internal namespace used in the schemas is also
//! changed to `http://schemas.openehr.org/v2`") — but the same release
//! RESTRUCTURED the repository from the flat `ALL/` bundle into per-component,
//! per-RM-release folders, and the older `Release-1.0.2v2` bundle was never
//! re-issued against a newer RM. So the v1 bundle is frozen at an RM
//! generation that predates many attributes the RM 1.2.0 codec always writes,
//! and it ships no `Ehr.xsd` and no `Demographic.xsd` at all. Consequently a
//! served v1 document CAN carry members its own schema does not declare; the
//! tests below enumerate exactly which, and prove each at the wire.

use openehr_its::json::from_canonical_value;
use openehr_its::xml::runtime::Namespace;
use openehr_its::xml::to_canonical_xml_ns;
use openehr_rm::prelude::{Composition, Folder};
use openehr_rm::v1_2::model;
use quick_xml::events::{BytesEnd, BytesStart, Event};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

// ---------------------------------------------------------------------------
// The vendored bundles
// ---------------------------------------------------------------------------

/// One vendored ITS-XML wire lineage: its namespace, the schema files that make
/// up its RM/BASE document set, and the global element names it publishes.
struct Bundle {
    /// Short label used in diagnostics and scratch-file names.
    label: &'static str,
    /// The lineage's XML target namespace.
    namespace: &'static str,
    /// The namespace this bundle's documents are serialized in.
    serialize_as: Namespace,
    /// Bundle files, relative to `crates/openehr-its/schemas/xml/`.
    files: &'static [&'static str],
    /// Global `xs:element` names the bundle itself declares (redeclaring one in
    /// the driver schema is a duplicate-declaration error).
    global_elements: &'static [&'static str],
}

/// The STABLE lineage (`http://schemas.openehr.org/v1`), upstream tag
/// `Release-1.0.2v2` — the flat `ALL/` packaging, which upstream publishes in
/// full as `components/ALL/` (11 XSDs) plus `components/AOM2/`.
/// `CompositionTemplate.xsd` is excluded because its target namespace is
/// `openEHR/v1/Template`, a different namespace that cannot be `xs:include`d
/// here; `AOM2/` is the archetype bundle, not the RM document set.
const V1: Bundle = Bundle {
    label: "nsv1",
    namespace: "http://schemas.openehr.org/v1",
    serialize_as: Namespace::V1,
    files: &[
        "its-xml-1.0.2-nsv1/ALL/Archetype.xsd",
        "its-xml-1.0.2-nsv1/ALL/BaseTypes.xsd",
        "its-xml-1.0.2-nsv1/ALL/Composition.xsd",
        "its-xml-1.0.2-nsv1/ALL/Content.xsd",
        "its-xml-1.0.2-nsv1/ALL/Extract.xsd",
        "its-xml-1.0.2-nsv1/ALL/OpenehrProfile.xsd",
        "its-xml-1.0.2-nsv1/ALL/Resource.xsd",
        "its-xml-1.0.2-nsv1/ALL/Structure.xsd",
        "its-xml-1.0.2-nsv1/ALL/Template.xsd",
        "its-xml-1.0.2-nsv1/ALL/Version.xsd",
    ],
    global_elements: &[
        "archetype",
        "composition",
        "extract",
        "extract_request",
        "items",
        "template",
        "version",
        "versioned_object",
    ],
};

/// The TRIAL lineage (`http://schemas.openehr.org/v2`), upstream tag
/// `Release-2.0.0v2` — the restructured `components/` tree, RM + BASE
/// `latest` (the RM document set the §XML Format link target resolves to).
/// It declares no global elements at all.
const V2: Bundle = Bundle {
    label: "nsv2",
    namespace: "http://schemas.openehr.org/v2",
    serialize_as: Namespace::V2,
    files: &[
        "its-xml-2.0.0-nsv2/RM/latest/Common.xsd",
        "its-xml-2.0.0-nsv2/RM/latest/DataStructures.xsd",
        "its-xml-2.0.0-nsv2/RM/latest/DataTypes.xsd",
        "its-xml-2.0.0-nsv2/RM/latest/Demographic.xsd",
        "its-xml-2.0.0-nsv2/RM/latest/Ehr.xsd",
        "its-xml-2.0.0-nsv2/RM/latest/EhrExtract.xsd",
        "its-xml-2.0.0-nsv2/BASE/latest/BaseTypes.xsd",
        "its-xml-2.0.0-nsv2/BASE/latest/Resource.xsd",
    ],
    global_elements: &[],
};

/// The vendored schema root.
fn schemas_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/xml")
}

/// A scratch directory for the driver schemas and serialized documents.
fn scratch() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openehr-its-xsd-gate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Write a driver schema for `bundle` declaring `roots` (`(element, type)`),
/// skipping any element name the bundle already publishes, and return its path.
fn write_driver(bundle: &Bundle, name: &str, roots: &[(&str, &str)]) -> PathBuf {
    use std::fmt::Write;
    let root = schemas_root();
    let mut xsd = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let ns = bundle.namespace;
    let _ = writeln!(
        xsd,
        "<xs:schema xmlns:xs=\"http://www.w3.org/2001/XMLSchema\" xmlns=\"{ns}\" \
         targetNamespace=\"{ns}\" elementFormDefault=\"qualified\">"
    );
    for f in bundle.files {
        let _ = writeln!(
            xsd,
            "  <xs:include schemaLocation=\"file://{}\"/>",
            root.join(f).display()
        );
    }
    for (element, ty) in roots {
        if bundle.global_elements.contains(element) {
            continue;
        }
        let _ = writeln!(xsd, "  <xs:element name=\"{element}\" type=\"{ty}\"/>");
    }
    xsd.push_str("</xs:schema>\n");
    let path = scratch().join(format!("{name}.xsd"));
    std::fs::write(&path, xsd).expect("write driver schema");
    path
}

/// The outcome of one `xmllint --schema` run.
#[derive(Debug)]
enum Outcome {
    /// The document validates against the bundle.
    Valid,
    /// The schema set itself failed to compile; carries the diagnostics.
    SchemaUncompilable(String),
    /// The schema compiled and rejected the document; carries the diagnostics.
    Invalid(String),
}

/// Run `xmllint --noout --schema <driver> <document>` and classify the result.
fn xmllint(driver: &Path, document: &Path) -> Outcome {
    let out = Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(driver)
        .arg(document)
        .output()
        .expect("run xmllint (this gate needs it, like the C14N gate)");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Outcome::Valid
    } else if stderr.contains("failed to compile") || stderr.contains("Schemas parser error") {
        Outcome::SchemaUncompilable(stderr)
    } else {
        Outcome::Invalid(stderr)
    }
}

// ---------------------------------------------------------------------------
// A minimal XSD reader: complexType -> declared member names
// ---------------------------------------------------------------------------

/// One `xs:complexType`, reduced to what the sweep needs.
#[derive(Default)]
struct XsdType {
    /// The `xs:extension`/`xs:restriction` `@base` this type derives from.
    base: Option<String>,
    /// Local `xs:element`/`xs:attribute` names declared inside the type.
    members: BTreeSet<String>,
}

/// The local part of a possibly-prefixed XML name.
fn local_name(qname: &str) -> &str {
    qname.rsplit(':').next().unwrap_or(qname)
}

/// Drop a `ns:` prefix from an XSD `QName`.
fn strip_prefix(qname: &str) -> String {
    qname.rsplit(':').next().unwrap_or(qname).to_string()
}

/// Parse a bundle's `xs:complexType` inventory (name -> base + own members).
fn read_types(bundle: &Bundle) -> BTreeMap<String, XsdType> {
    let root = schemas_root();
    let mut types: BTreeMap<String, XsdType> = BTreeMap::new();
    for f in bundle.files {
        let path = root.join(f);
        let xml = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        read_types_from(&xml, &mut types);
    }
    types
}

/// Read one XSD document's `xs:complexType` inventory into `types`.
///
/// Only a TOP-LEVEL `complexType` (depth 1) names a type; nested anonymous
/// ones contribute their members to the enclosing named type.
fn read_types_from(xml: &str, types: &mut BTreeMap<String, XsdType>) {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut current: Option<String> = None;
    let mut depth = 0usize;
    loop {
        let ev = reader.read_event().expect("well-formed vendored XSD");
        match &ev {
            Event::Start(e) => record_element(e, false, &mut current, &mut depth, types),
            Event::Empty(e) => record_element(e, true, &mut current, &mut depth, types),
            Event::End(e) => close_element(e, &mut current, &mut depth),
            Event::Eof => return,
            _ => {}
        }
    }
}

/// Reads one attribute of `element` by local name, prefix-insensitively.
fn xsd_attr(element: &BytesStart<'_>, key: &str) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| local_name(a.key.as_ref()) == key)
        .map(|a| a.value.as_ref().to_owned())
}

/// Leaves the `complexType` the reader is closing, clearing the current type
/// name once the outermost one ends.
fn close_element(element: &BytesEnd<'_>, current: &mut Option<String>, depth: &mut usize) {
    if local_name(element.name().as_ref()) != "complexType" {
        return;
    }
    *depth = depth.saturating_sub(1);
    if *depth == 0 {
        *current = None;
    }
}

/// Folds one opening (or self-closing) XSD element into `types`.
///
/// `empty` distinguishes `<xs:complexType/>` from `<xs:complexType>`, which is
/// what keeps the nesting depth honest.
fn record_element(
    element: &BytesStart<'_>,
    empty: bool,
    current: &mut Option<String>,
    depth: &mut usize,
    types: &mut BTreeMap<String, XsdType>,
) {
    match local_name(element.name().as_ref()) {
        "complexType" => open_complex_type(element, empty, current, depth, types),
        "extension" | "restriction" => record_base(element, current.as_deref(), types),
        "element" | "attribute" => record_member(element, current.as_deref(), types),
        _ => {}
    }
}

/// Enters an `xs:complexType`, naming it only at depth 1 — a nested anonymous
/// one contributes its members to the enclosing named type instead.
fn open_complex_type(
    element: &BytesStart<'_>,
    empty: bool,
    current: &mut Option<String>,
    depth: &mut usize,
    types: &mut BTreeMap<String, XsdType>,
) {
    if !empty {
        *depth += 1;
    }
    if *depth != 1 {
        return;
    }
    *current = xsd_attr(element, "name");
    if let Some(n) = current {
        types.entry(n.clone()).or_default();
    }
}

/// Records the current type's `xs:extension`/`xs:restriction` `@base`. The
/// first one wins: an inner derivation never overwrites the outer type's base.
fn record_base(
    element: &BytesStart<'_>,
    current: Option<&str>,
    types: &mut BTreeMap<String, XsdType>,
) {
    let (Some(n), Some(b)) = (current, xsd_attr(element, "base")) else {
        return;
    };
    let entry = types.entry(n.to_owned()).or_default();
    if entry.base.is_none() {
        entry.base = Some(strip_prefix(&b));
    }
}

/// Records one `xs:element`/`xs:attribute` name as a member of the current type.
fn record_member(
    element: &BytesStart<'_>,
    current: Option<&str>,
    types: &mut BTreeMap<String, XsdType>,
) {
    if let (Some(n), Some(m)) = (current, xsd_attr(element, "name")) {
        types.entry(n.to_owned()).or_default().members.insert(m);
    }
}

/// Flattened (own + inherited) member names of `name`.
fn flattened(types: &BTreeMap<String, XsdType>, name: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut cur: Option<&str> = Some(name);
    while let Some(n) = cur {
        if !seen.insert(n) {
            break;
        }
        let Some(t) = types.get(n) else { break };
        out.extend(t.members.iter().cloned());
        cur = t.base.as_deref();
    }
    out
}

/// A concrete, non-enumeration RM/BASE class named the openEHR way. Abstract
/// classes are excluded because the codec never writes one under its own name;
/// enumerations are excluded because the XSDs model them as `xs:simpleType`,
/// not `xs:complexType`; foundation types (`Interval`, `Iso8601_date`, …) are
/// excluded by the `SCREAMING_SNAKE` naming test — the XSDs give them bespoke
/// spellings (`IntervalOfInteger`, `xs:string`) rather than a class of their
/// own.
fn is_wire_class(name: &str) -> bool {
    name.chars()
        .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
        && model::class(name).is_some_and(|c| !c.is_abstract && c.generated_type)
        && model::enumeration(name).is_none()
}

// ---------------------------------------------------------------------------
// 1. The nsv2 lineage cannot be compiled by a conformant XSD processor
// ---------------------------------------------------------------------------

/// The nsv2 bundle's `archetypeNodeId` facet is not a valid XML Schema regular
/// expression, so NO conformant processor can compile the lineage — and no
/// document can therefore be validated against it.
///
/// `its-xml-2.0.0-nsv2/BASE/latest/BaseTypes.xsd` (`xs:simpleType
/// archetypeNodeId`) carries a Perl-flavoured pattern built from `(?:…)`
/// non-capturing groups, with an upstream comment pointing at regex101. XML
/// Schema Part 2 Appendix F defines the regular-expression language for the
/// `pattern` facet, and its `atom` production admits only a character, a
/// character class, or a parenthesised `regExp` — there is no `(?…)` construct
/// at all (<https://www.w3.org/TR/xmlschema-2/#regexs>). libxml2 rejects it,
/// as any conformant implementation must. The same defective facet is present
/// in `BASE/Release-1.1.0`, `BASE/Release-1.2.0` and the nsv2 `RM/Release-1.0.2`
/// plus `RM/Release-1.0.3` local `BaseTypes.xsd`, and every nsv2 RM release
/// folder includes one of them — so the defect covers the whole lineage, not
/// just `latest`. The v1 bundle's own `archetypeNodeId` facet
/// (`its-xml-1.0.2-nsv1/ALL/BaseTypes.xsd`) is plain XSD regex and compiles.
///
/// This is pinned rather than worked around: the moment upstream fixes the
/// facet (or the bundle is re-vendored) this test fails, and the nsv2 half of
/// the gate can be promoted from "uncompilable" to real validation.
#[test]
fn nsv2_lineage_is_uncompilable_by_a_conformant_xsd_processor() {
    let driver = write_driver(&V2, "nsv2_compile_probe", &[("folder", "FOLDER")]);
    let document = scratch().join("compile_probe.xml");
    std::fs::write(&document, "<probe/>\n").expect("write probe document");
    match xmllint(&driver, &document) {
        Outcome::SchemaUncompilable(diagnostics) => {
            assert!(
                diagnostics.contains("BASE/latest/BaseTypes.xsd"),
                "the compile failure must come from the vendored BaseTypes facet: {diagnostics}"
            );
            assert!(
                diagnostics.contains("is not a valid regular expression"),
                "the compile failure must be the pattern facet: {diagnostics}"
            );
        }
        other => panic!(
            "the nsv2 bundle now compiles ({other:?}) — re-adjudicate: promote the nsv2 vectors \
             in DOCUMENT_VECTORS from Expect::Uncompilable to a real expectation"
        ),
    }
}

// ---------------------------------------------------------------------------
// 2. RM classes a bundle publishes no complexType for
// ---------------------------------------------------------------------------

/// Concrete RM 1.2.0 classes with NO `xs:complexType` in the nsv1 bundle.
///
/// The `Release-1.0.2v2` tag publishes `components/ALL/` (11 XSDs) +
/// `components/AOM2/` and nothing else — verified against the upstream tag
/// tree — so the whole EHR package (`Ehr.xsd`) and the whole demographic
/// package (`Demographic.xsd`) are simply absent, together with everything RM
/// added afterwards (`DV_SCALE`, `ITEM_TAG`, the `VERSIONED_*` containers).
/// A canonical-XML document of any of these has no v1 schema to conform to at
/// all — the media type is declared, the schema binding is not.
const V1_ABSENT_TYPES: &[&str] = &[
    "ADDRESS",
    "ADDRESSED_MESSAGE",
    "AGENT",
    "CAPABILITY",
    "CODE_SET_ACCESS",
    "CONTACT",
    "CONTRIBUTION",
    "DV_SCALE",
    "EHR",
    "EHR_ACCESS",
    "EHR_STATUS",
    "EXTRACT_ACTION_REQUEST",
    "EXTRACT_ENTITY_CHAPTER",
    "EXTRACT_ERROR",
    "EXTRACT_PARTICIPATION",
    "GENERIC_CONTENT_ITEM",
    "GROUP",
    "INTERNET_ID",
    "ISO_OID",
    "ITEM_TAG",
    "MEASUREMENT_SERVICE",
    "MESSAGE",
    "OPENEHR_CODE_SET_IDENTIFIERS",
    "OPENEHR_CONTENT_ITEM",
    "OPENEHR_TERMINOLOGY_GROUP_IDENTIFIERS",
    "ORGANISATION",
    "PARTY_IDENTITY",
    "PARTY_RELATIONSHIP",
    "PERSON",
    "RESOURCE_ANNOTATIONS",
    "ROLE",
    "SYNC_EXTRACT",
    "SYNC_EXTRACT_REQUEST",
    "SYNC_EXTRACT_SPEC",
    "TERMINOLOGY_ACCESS",
    "TERMINOLOGY_SERVICE",
    "UUID",
    "VERSIONED_COMPOSITION",
    "VERSIONED_EHR_ACCESS",
    "VERSIONED_EHR_STATUS",
    "VERSIONED_FOLDER",
    "VERSIONED_OBJECT",
    "VERSIONED_PARTY",
    "VERSION_TREE_ID",
    "X_CONTRIBUTION",
    "X_VERSIONED_COMPOSITION",
    "X_VERSIONED_EHR_ACCESS",
    "X_VERSIONED_EHR_STATUS",
    "X_VERSIONED_FOLDER",
    "X_VERSIONED_PARTY",
];

/// Concrete RM 1.2.0 classes with NO `xs:complexType` in the nsv2 RM+BASE
/// `latest` bundle. `ITEM_TAG` and `EXTRACT_ERROR` are RM classes the published
/// schemas never modelled; the rest are the service/terminology-access
/// interfaces (`TERMINOLOGY_SERVICE`, `MEASUREMENT_SERVICE`, …) and the two
/// `UID` subtypes the XSDs express as `xs:token` facets rather than types —
/// none of them are document content.
const V2_ABSENT_TYPES: &[&str] = &[
    "CODE_SET_ACCESS",
    "EXTRACT_ERROR",
    "INTERNET_ID",
    "ISO_OID",
    "ITEM_TAG",
    "MEASUREMENT_SERVICE",
    "OPENEHR_CODE_SET_IDENTIFIERS",
    "OPENEHR_TERMINOLOGY_GROUP_IDENTIFIERS",
    "TERMINOLOGY_ACCESS",
    "TERMINOLOGY_SERVICE",
];

#[test]
fn absent_types_match_the_adjudicated_set() {
    for (bundle, pinned) in [(&V1, V1_ABSENT_TYPES), (&V2, V2_ABSENT_TYPES)] {
        let types = read_types(bundle);
        let computed: BTreeSet<&str> = model::classes()
            .map(|c| c.name)
            .filter(|n| is_wire_class(n) && !types.contains_key(*n))
            .collect();
        let expected: BTreeSet<&str> = pinned.iter().copied().collect();
        assert_eq!(
            computed,
            expected,
            "{}: the set of RM classes with no complexType changed.\n  newly absent: {:?}\n  \
             no longer absent: {:?}\nRe-adjudicate V1_ABSENT_TYPES / V2_ABSENT_TYPES against the \
             vendored bundle before touching this list.",
            bundle.label,
            computed.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&computed).collect::<Vec<_>>()
        );
    }
}

// ---------------------------------------------------------------------------
// 3. RM attributes a bundle's complexType does not declare
// ---------------------------------------------------------------------------

/// `(class, attribute, adjudication)` — every RM 1.2.0 attribute the nsv1
/// bundle's own complexType (flattened over its `xs:extension` chain) does not
/// declare. Each entry names the nsv2 RM release folder that DOES declare it,
/// which is the schema delta: the v1 bundle is frozen at an RM generation older
/// than the codec's RM 1.2.0 model. Only classes the bundle declares at all
/// appear here; classes it omits entirely are `V1_ABSENT_TYPES`.
const V1_MEMBER_DIVERGENCES: &[(&str, &str, &str)] = &[
    (
        "ACTION",
        "workflow_id",
        "ENTRY.workflow_id — declared on ENTRY by every nsv2 RM release folder \
         (RM/Release-1.0.2/Ehr.xsd onward); the flat nsv1 ALL/Content.xsd omits it entirely",
    ),
    (
        "ADMIN_ENTRY",
        "workflow_id",
        "ENTRY.workflow_id — as ACTION.workflow_id",
    ),
    (
        "CODE_PHRASE",
        "preferred_term",
        "added in RM/Release-1.1.0/BaseTypes.xsd (nsv2); absent from nsv1 ALL/BaseTypes.xsd",
    ),
    (
        "DV_QUANTITY",
        "units_system",
        "added in RM/Release-1.1.0/DataTypes.xsd (nsv2); absent from nsv1 ALL/BaseTypes.xsd",
    ),
    (
        "DV_QUANTITY",
        "units_display_name",
        "added in RM/Release-1.1.0/DataTypes.xsd (nsv2); absent from nsv1 ALL/BaseTypes.xsd",
    ),
    (
        "ELEMENT",
        "null_reason",
        "added in RM/Release-1.1.0/DataStructures.xsd (nsv2); nsv1 ALL/Structure.xsd declares \
         null_flavour alone",
    ),
    (
        "EVALUATION",
        "workflow_id",
        "ENTRY.workflow_id — as ACTION.workflow_id",
    ),
    (
        "EXTRACT_CHAPTER",
        "items",
        "added in RM/Release-1.0.3/EhrExtract.xsd (nsv2); absent from nsv1 ALL/Extract.xsd",
    ),
    (
        "EXTRACT_ENTITY_MANIFEST",
        "extract_id_key",
        "added in RM/Release-1.0.3/EhrExtract.xsd (nsv2); absent from nsv1 ALL/Extract.xsd",
    ),
    (
        "EXTRACT_ENTITY_MANIFEST",
        "ehr_id",
        "added in RM/Release-1.0.3/EhrExtract.xsd (nsv2); absent from nsv1 ALL/Extract.xsd",
    ),
    (
        "EXTRACT_ENTITY_MANIFEST",
        "subject_id",
        "added in RM/Release-1.0.3/EhrExtract.xsd (nsv2); absent from nsv1 ALL/Extract.xsd",
    ),
    (
        "EXTRACT_ENTITY_MANIFEST",
        "other_ids",
        "added in RM/Release-1.0.3/EhrExtract.xsd (nsv2); absent from nsv1 ALL/Extract.xsd",
    ),
    (
        "EXTRACT_SPEC",
        "priority",
        "added in RM/Release-1.0.3/EhrExtract.xsd (nsv2); absent from nsv1 ALL/Extract.xsd",
    ),
    (
        "EXTRACT_SPEC",
        "include_multimedia",
        "SPELLING: every published XSD (both lineages) spells it includes_multimedia; RM ehr_extract \
         master04-common_package.adoc §EXTRACT_SPEC and \
         UML/classes/org.openehr.rm.ehr_extract.extract_spec.adoc both spell it include_multimedia, \
         which is what the RM model and the codec emit",
    ),
    (
        "EXTRACT_UPDATE_SPEC",
        "update_method",
        "added in RM/Release-1.0.3/EhrExtract.xsd (nsv2); absent from nsv1 ALL/Extract.xsd",
    ),
    (
        "EXTRACT_VERSION_SPEC",
        "include_revision_history",
        "SPELLING: both lineages spell it includes_revision_history; RM \
         UML/classes/org.openehr.rm.ehr_extract.extract_version_spec.adoc spells it \
         include_revision_history",
    ),
    (
        "EXTRACT_VERSION_SPEC",
        "include_data",
        "SPELLING: both lineages spell it includes_data; RM \
         UML/classes/org.openehr.rm.ehr_extract.extract_version_spec.adoc spells it include_data",
    ),
    (
        "FEEDER_AUDIT_DETAILS",
        "other_details",
        "added in RM/Release-1.1.0/Common.xsd (nsv2); absent from nsv1 ALL/BaseTypes.xsd",
    ),
    (
        "FOLDER",
        "details",
        "added in RM/Release-1.1.0/Common.xsd (nsv2, whose DataStructures.xsd include carries the \
         upstream comment \"this is required by FOLDER.details\"); nsv1 ALL/Structure.xsd declares \
         folders + items alone. RM common master05-directory_package.adoc §Overview: \"Any \
         individual Folder may contain meta-data in its details attribute (type ITEM_STRUCTURE)\"",
    ),
    (
        "INSTRUCTION",
        "workflow_id",
        "ENTRY.workflow_id — as ACTION.workflow_id",
    ),
    (
        "ISM_TRANSITION",
        "reason",
        "added in RM/Release-1.0.3/Ehr.xsd (nsv2); absent from nsv1 ALL/Content.xsd",
    ),
    (
        "OBSERVATION",
        "workflow_id",
        "ENTRY.workflow_id — as ACTION.workflow_id",
    ),
    (
        "TRANSLATION_DETAILS",
        "accreditaton",
        "SPELLING, ours: both lineages spell it accreditation, and so does BASE \
         UML/classes/org.openehr.base.resource.translation_details.adoc — BASE \
         resource/master00-amendment_record.adoc records SPECPUB-6 \"Correct spelling error in \
         TRANSLATION_DETAILS._accreditation_\". openehr-base carries the corrected spelling; the \
         RM BMM's own stale copy of the resource package still carries the typo, and that copy is \
         what openehr_rm::v1_2::common::resource emits",
    ),
];

/// The same sweep against the nsv2 RM+BASE `latest` bundle. Every entry here is
/// a divergence between the RM 1.2.0 model and the newest published schema —
/// not a lineage-age effect — so each is either a spec-text-vs-XSD spelling
/// conflict or an RM addition the XSDs have not caught up with.
const V2_MEMBER_DIVERGENCES: &[(&str, &str, &str)] = &[
    (
        "EHR",
        "tags",
        "RM UML/classes/org.openehr.rm.ehr.ehr.adoc declares EHR.tags (List<OBJECT_REF>, \
         \"Optional list of tags associated with this EHR\"); RM/latest/Ehr.xsd does not declare it",
    ),
    (
        "EXTRACT_SPEC",
        "include_multimedia",
        "SPELLING: RM/latest/EhrExtract.xsd spells it includes_multimedia, the RM text \
         include_multimedia — see the nsv1 entry",
    ),
    (
        "EXTRACT_VERSION_SPEC",
        "include_revision_history",
        "SPELLING: RM/latest/EhrExtract.xsd spells it includes_revision_history — see the nsv1 entry",
    ),
    (
        "EXTRACT_VERSION_SPEC",
        "include_data",
        "SPELLING: RM/latest/EhrExtract.xsd spells it includes_data — see the nsv1 entry",
    ),
    (
        "RESOURCE_DESCRIPTION_ITEM",
        "copyright",
        "PLACEMENT, ours: BASE UML/classes/org.openehr.base.resource.resource_description.adoc puts \
         copyright on RESOURCE_DESCRIPTION, and BASE/latest/Resource.xsd agrees; the RM BMM's stale \
         copy of the resource package puts it on RESOURCE_DESCRIPTION_ITEM, and that copy is what \
         openehr_rm::v1_2::common::resource emits",
    ),
    (
        "TRANSLATION_DETAILS",
        "accreditaton",
        "SPELLING, ours: see the nsv1 entry (SPECPUB-6)",
    ),
];

#[test]
fn member_divergences_match_the_adjudicated_set() {
    for (bundle, pinned) in [(&V1, V1_MEMBER_DIVERGENCES), (&V2, V2_MEMBER_DIVERGENCES)] {
        let types = read_types(bundle);
        let mut computed: BTreeSet<(&str, &str)> = BTreeSet::new();
        for class in model::classes() {
            if !is_wire_class(class.name) || !types.contains_key(class.name) {
                continue;
            }
            let declared = flattened(&types, class.name);
            for attribute in class.attributes {
                if !declared.contains(attribute.name) {
                    computed.insert((class.name, attribute.name));
                }
            }
        }
        let expected: BTreeSet<(&str, &str)> = pinned.iter().map(|(c, a, _)| (*c, *a)).collect();
        assert_eq!(
            computed,
            expected,
            "{}: the RM-model-vs-XSD member delta changed.\n  new divergences: {:?}\n  \
             resolved divergences: {:?}\nEvery entry needs its own adjudication (the schema delta \
             or the spec-text conflict) before it may be added.",
            bundle.label,
            computed.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&computed).collect::<Vec<_>>()
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Served documents against both lineages
// ---------------------------------------------------------------------------

/// The corpus COMPOSITION every composition vector is built from.
fn base_composition() -> Value {
    serde_json::from_str(include_str!(
        "../vendor/openehr_sdk/composition/canonical_json/minimal_evaluation.json"
    ))
    .expect("corpus composition JSON")
}

/// A directory-root FOLDER — the shape the `/ehr/{id}/directory` routes serve.
fn base_folder() -> Value {
    json!({
        "_type": "FOLDER",
        "name": { "_type": "DV_TEXT", "value": "root" },
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1"
    })
}

/// What a vector is expected to do against a bundle.
enum Expect {
    /// The served document conforms to the bundle's schema.
    Valid,
    /// The schema rejects the document; the diagnostics MUST contain `needle`,
    /// so the vector cannot start failing for an unrelated reason and stay
    /// green. `adjudication` names the schema delta that explains it.
    Invalid {
        /// A substring the `xmllint` diagnostic must contain.
        needle: &'static str,
        /// The adjudicated reason the served document diverges.
        adjudication: &'static str,
    },
    /// The bundle's schema set cannot be compiled at all — see
    /// [`nsv2_lineage_is_uncompilable_by_a_conformant_xsd_processor`].
    Uncompilable,
}

/// One serialized document under test.
struct Vector {
    /// Diagnostic label.
    label: &'static str,
    /// The document root element name the REST surface serves it under.
    root: &'static str,
    /// The XSD type the driver schema binds that root to.
    rm_type: &'static str,
    /// The canonical JSON the document is serialized from.
    value: Value,
    /// The expected nsv1 outcome.
    v1: Expect,
    /// The expected nsv2 outcome — uniformly `Uncompilable` while the lineage's
    /// `archetypeNodeId` pattern facet stays invalid.
    v2: Expect,
}

/// Every served-document vector, each exercising one nsv1 divergence (or the
/// baseline that must stay clean).
fn vectors() -> Vec<Vector> {
    let mut all = composition_vectors();
    all.extend(composition_vectors_continued());
    all.extend(folder_vectors());
    all
}

/// The corpus COMPOSITION with one mutation applied — the shape of every
/// divergence vector: a single added member on an otherwise clean document, so
/// a rejection can only be about that member.
fn compo_with(mutate: impl FnOnce(&mut Value)) -> Value {
    let mut value = base_composition();
    mutate(&mut value);
    value
}

/// The COMPOSITION vectors: the clean baseline plus one per nsv1 divergence
/// reachable inside composition content.
fn composition_vectors() -> Vec<Vector> {
    vec![
        Vector {
            label: "composition.baseline",
            root: "composition",
            rm_type: "COMPOSITION",
            value: base_composition(),
            v1: Expect::Valid,
            v2: Expect::Uncompilable,
        },
        Vector {
            label: "composition.entry_workflow_id",
            root: "composition",
            rm_type: "COMPOSITION",
            value: compo_with(|c| {
                c["content"][0]["workflow_id"] = json!({
                    "_type": "OBJECT_REF",
                    "namespace": "local",
                    "type": "INSTRUCTION",
                    "id": { "_type": "HIER_OBJECT_ID",
                        "value": "0198f4a5-9df1-7d1e-8b6f-2b8c00000001" }
                });
            }),
            v1: Expect::Invalid {
                needle: "}workflow_id'",
                adjudication: "ENTRY.workflow_id is undeclared in nsv1 ALL/Content.xsd",
            },
            v2: Expect::Uncompilable,
        },
        Vector {
            label: "composition.element_null_reason",
            root: "composition",
            rm_type: "COMPOSITION",
            value: compo_with(|c| {
                c["content"][0]["data"]["items"][0]["null_reason"] =
                    json!({ "_type": "DV_TEXT", "value": "not recorded" });
            }),
            v1: Expect::Invalid {
                needle: "}null_reason'",
                adjudication: "ELEMENT.null_reason is an RM 1.1.0 addition",
            },
            v2: Expect::Uncompilable,
        },
        Vector {
            label: "composition.code_phrase_preferred_term",
            root: "composition",
            rm_type: "COMPOSITION",
            value: compo_with(|c| {
                c["language"]["preferred_term"] = json!("English");
            }),
            v1: Expect::Invalid {
                needle: "}preferred_term'",
                adjudication: "CODE_PHRASE.preferred_term is an RM 1.1.0 addition",
            },
            v2: Expect::Uncompilable,
        },
    ]
}

/// The remaining COMPOSITION vectors (kept separate so no single builder grows
/// unreadably long): the RM 1.1.0 data-type/audit additions and the RM class
/// the nsv1 bundle has no type for at all.
fn composition_vectors_continued() -> Vec<Vector> {
    vec![
        Vector {
            label: "composition.dv_quantity_units_system",
            root: "composition",
            rm_type: "COMPOSITION",
            value: compo_with(|c| {
                let quantity = &mut c["content"][0]["data"]["items"][0]["value"];
                quantity["units_system"] = json!("http://unitsofmeasure.org");
                quantity["units_display_name"] = json!("kg");
            }),
            v1: Expect::Invalid {
                needle: "}units_system'",
                adjudication: "DV_QUANTITY.units_system/units_display_name are RM 1.1.0 additions",
            },
            v2: Expect::Uncompilable,
        },
        Vector {
            label: "composition.feeder_audit_other_details",
            root: "composition",
            rm_type: "COMPOSITION",
            value: compo_with(|c| {
                c["feeder_audit"] = json!({
                    "_type": "FEEDER_AUDIT",
                    "originating_system_audit": {
                        "_type": "FEEDER_AUDIT_DETAILS",
                        "system_id": "legacy-lims",
                        "other_details": {
                            "_type": "ITEM_TREE",
                            "name": { "_type": "DV_TEXT", "value": "extra" },
                            "archetype_node_id": "at0001",
                            "items": [ { "_type": "ELEMENT",
                                "name": { "_type": "DV_TEXT", "value": "note" },
                                "archetype_node_id": "at0002",
                                "value": { "_type": "DV_TEXT", "value": "imported" } } ]
                        }
                    }
                });
            }),
            v1: Expect::Invalid {
                needle: "}other_details'",
                adjudication: "FEEDER_AUDIT_DETAILS.other_details is an RM 1.1.0 addition",
            },
            v2: Expect::Uncompilable,
        },
        Vector {
            label: "composition.dv_scale",
            root: "composition",
            rm_type: "COMPOSITION",
            value: compo_with(|c| {
                c["content"][0]["data"]["items"][0]["value"] = json!({
                    "_type": "DV_SCALE",
                    "symbol": { "_type": "DV_CODED_TEXT", "value": "mild",
                        "defining_code": { "_type": "CODE_PHRASE",
                            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "local" },
                            "code_string": "at0005" } },
                    "value": 1.0
                });
            }),
            v1: Expect::Invalid {
                needle: "}DV_SCALE' of the xsi:type attribute does not resolve",
                adjudication: "DV_SCALE has no complexType in the nsv1 bundle at all",
            },
            v2: Expect::Uncompilable,
        },
    ]
}

/// The FOLDER (EHR directory) vectors: the clean baseline plus the
/// `FOLDER.details` divergence the CDR actually serves.
fn folder_vectors() -> Vec<Vector> {
    let mut details = base_folder();
    details["details"] = json!({
        "_type": "ITEM_TREE",
        "name": { "_type": "DV_TEXT", "value": "Tree" },
        "archetype_node_id": "at0003",
        "items": [ { "_type": "ELEMENT",
            "name": { "_type": "DV_TEXT", "value": "text" },
            "archetype_node_id": "at0004",
            "value": { "_type": "DV_TEXT", "value": "ward 4" } } ]
    });

    vec![
        Vector {
            label: "directory.baseline",
            root: "folder",
            rm_type: "FOLDER",
            value: base_folder(),
            v1: Expect::Valid,
            v2: Expect::Uncompilable,
        },
        Vector {
            label: "directory.details",
            root: "folder",
            rm_type: "FOLDER",
            value: details,
            v1: Expect::Invalid {
                needle: "}details'",
                adjudication: "FOLDER.details is an RM 1.1.0 addition; this is the shape \
                               corpus/fixtures/directory/v2.xml carries",
            },
            v2: Expect::Uncompilable,
        },
    ]
}

/// Serialize one vector into `bundle`'s lineage and validate it.
fn validate(bundle: &Bundle, vector: &Vector) -> Outcome {
    let xml = match vector.rm_type {
        "COMPOSITION" => {
            let typed: Composition =
                from_canonical_value(&vector.value).expect("typed COMPOSITION");
            to_canonical_xml_ns(&typed, vector.root, bundle.serialize_as)
        }
        "FOLDER" => {
            let typed: Folder = from_canonical_value(&vector.value).expect("typed FOLDER");
            to_canonical_xml_ns(&typed, vector.root, bundle.serialize_as)
        }
        other => panic!("vector {}: unhandled RM type {other}", vector.label),
    }
    .expect("serialize canonical XML");

    let driver = write_driver(
        bundle,
        &format!("{}_{}", bundle.label, vector.root),
        &[(vector.root, vector.rm_type)],
    );
    let document = scratch().join(format!("{}.{}.xml", bundle.label, vector.label));
    std::fs::write(&document, &xml).expect("write document");
    xmllint(&driver, &document)
}

#[test]
fn served_documents_validate_or_carry_an_adjudicated_divergence() {
    for vector in vectors() {
        match (validate(&V1, &vector), &vector.v1) {
            (Outcome::Valid, Expect::Valid) => {}
            (
                Outcome::Invalid(diagnostics),
                Expect::Invalid {
                    needle,
                    adjudication,
                },
            ) => {
                assert!(
                    diagnostics.contains(needle),
                    "{}: rejected for a DIFFERENT reason than the adjudicated one \
                     ({adjudication}); expected the diagnostic to contain {needle:?}, got:\n{diagnostics}",
                    vector.label
                );
            }
            (actual, _) => panic!(
                "{}: nsv1 outcome changed to {actual:?} — re-adjudicate the vector against \
                 crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/ALL/ before editing the \
                 expectation",
                vector.label
            ),
        }
        match (validate(&V2, &vector), &vector.v2) {
            (Outcome::SchemaUncompilable(_), Expect::Uncompilable)
            | (Outcome::Valid, Expect::Valid) => {}
            (
                Outcome::Invalid(diagnostics),
                Expect::Invalid {
                    needle,
                    adjudication,
                },
            ) => {
                assert!(
                    diagnostics.contains(needle),
                    "{}: rejected for a DIFFERENT reason than the adjudicated one \
                     ({adjudication}); expected the diagnostic to contain {needle:?}, \
                     got:\n{diagnostics}",
                    vector.label
                );
            }
            (actual, _) => panic!(
                "{}: nsv2 outcome changed to {actual:?} — promote the nsv2 expectations from \
                 Uncompilable to real per-vector outcomes",
                vector.label
            ),
        }
    }
}
