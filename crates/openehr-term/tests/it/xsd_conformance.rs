// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The vendored terminology assets conform to the vendored XSDs.
//!
//! TERM `SupportTerminology/master04-representation.adoc` §XML Representation:
//! the concrete representation of the code sets and vocabularies is "the XML
//! format described by the XML Schema found in the openEHR
//! `specifications-TERM` repository", and "An XML Schema (XSD) has been
//! defined for these files, for use with software that processes them". The
//! three schemas are vendored at `assets/schema/` (byte-identical to upstream
//! at the pin — `assets/PROVENANCE.md`); no pure-Rust XSD validator is in the
//! pinned dependency set, so this suite mirrors the schemas' structural rules
//! directly — every assertion cites the XSD element/attribute declaration it
//! encodes — and walks all seven vendored assets.

/// The five language bundles governed by `assets/schema/openehr_terminology.xsd`.
const LANGUAGE_ASSETS: [(&str, &str); 5] = [
    (
        "en",
        include_str!("../../assets/en/openehr_terminology.xml"),
    ),
    (
        "es",
        include_str!("../../assets/es/openehr_terminology.xml"),
    ),
    (
        "ja",
        include_str!("../../assets/ja/openehr_terminology.xml"),
    ),
    (
        "pt",
        include_str!("../../assets/pt/openehr_terminology.xml"),
    ),
    (
        "zh",
        include_str!("../../assets/zh/openehr_terminology.xml"),
    ),
];

const EXTERNAL_XML: &str = include_str!("../../assets/openehr_external_terminologies.xml");
const PROPERTY_UNITS_XML: &str = include_str!("../../assets/PropertyUnitData.xml");

/// `xs:NCName` per the XSD datatypes spec as the schemas use it: no colon, no
/// whitespace, and the first character is a letter or underscore (a digit,
/// dot, or minus may appear only later).
fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_')
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// `xs:date` in the lexical form the schemas' data uses: `YYYY-MM-DD`.
fn is_xs_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.get(4) == Some(&b'-')
        && b.get(7) == Some(&b'-')
        && s.char_indices()
            .all(|(i, c)| matches!(i, 4 | 7) || c.is_ascii_digit())
}

/// Assert `node` declares no attribute outside `allowed` and every name in
/// `required` — `xs:complexType` admits only its declared attributes.
fn check_attrs(label: &str, node: &roxmltree::Node, required: &[&str], optional: &[&str]) {
    for want in required {
        assert!(
            node.attribute(*want).is_some(),
            "{label}: required attribute {want:?} missing (use=\"required\" in the XSD)"
        );
    }
    for a in node.attributes() {
        let name = a.name();
        assert!(
            required.contains(&name) || optional.contains(&name),
            "{label}: attribute {name:?} is not declared by the XSD"
        );
    }
}

/// The `terminology` document shape shared by both terminology schemas:
/// root attributes (`language` + `name` required `NCName`, `version` string,
/// `date` `xs:date`), `codeset` (1..*) elements each holding `code` (1..*),
/// and — when `groups_allowed` (`openehr_terminology.xsd`) — `group` (1..*)
/// elements each holding `concept` (1..*), with every codeset preceding
/// every group (`xs:sequence`).
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "a failed lookup after its own presence assertion, and an undeclared element, ARE the test failure (Book ch11 assertion idiom)"
)]
fn check_terminology_doc(label: &str, xml: &str, groups_allowed: bool, external_id_required: bool) {
    let doc = roxmltree::Document::parse(xml).expect("well-formed XML");
    let root = doc.root_element();
    assert_eq!(
        root.tag_name().name(),
        "terminology",
        "{label}: root element"
    );
    check_attrs(
        &format!("{label}/terminology"),
        &root,
        &["language", "name"],
        &["version", "date"],
    );
    for a in ["language", "name"] {
        let v = root.attribute(a).expect("checked required above");
        assert!(
            is_ncname(v),
            "{label}: terminology@{a}={v:?} is not an NCName"
        );
    }
    if let Some(d) = root.attribute("date") {
        assert!(
            is_xs_date(d),
            "{label}: terminology@date={d:?} is not an xs:date"
        );
    }

    let mut seen_codeset = false;
    let mut seen_group = false;
    for child in root.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "codeset" => {
                assert!(
                    !seen_group,
                    "{label}: <codeset> after <group> — the xs:sequence puts every codeset first"
                );
                seen_codeset = true;
                check_codeset(label, &child, external_id_required);
            }
            "group" => {
                assert!(
                    groups_allowed,
                    "{label}: <group> is not declared by this schema"
                );
                seen_group = true;
                check_group(label, &child);
            }
            other => panic!("{label}: element <{other}> is not declared by the schema"),
        }
    }
    assert!(
        seen_codeset,
        "{label}: <codeset> is minOccurs>=1 in the sequence"
    );
    if groups_allowed {
        assert!(
            seen_group,
            "{label}: <group> is minOccurs>=1 in the sequence"
        );
    }
}

/// One `<codeset>` element and its `<code>` children against the XSD.
#[expect(
    clippy::expect_used,
    reason = "a failed lookup after its own presence assertion IS the test failure (Book ch11 assertion idiom)"
)]
fn check_codeset(label: &str, child: &roxmltree::Node<'_, '_>, external_id_required: bool) {
    let cs_label = format!(
        "{label}/codeset[{}]",
        child.attribute("openehr_id").unwrap_or("?")
    );
    let required: &[&str] = if external_id_required {
        // openehr_external_terminologies.xsd: external_id use="required".
        &["issuer", "openehr_id", "external_id"]
    } else {
        &["issuer", "openehr_id"]
    };
    check_attrs(
        &cs_label,
        child,
        required,
        &["external_id", "name", "status"],
    );
    for a in ["issuer", "openehr_id"] {
        let v = child.attribute(a).expect("checked required above");
        assert!(is_ncname(v), "{cs_label}: @{a}={v:?} is not an NCName");
    }
    if let Some(v) = child.attribute("external_id") {
        assert!(
            is_ncname(v),
            "{cs_label}: @external_id={v:?} is not an NCName"
        );
    }
    let mut codes = 0usize;
    for code in child.children().filter(roxmltree::Node::is_element) {
        assert_eq!(
            code.tag_name().name(),
            "code",
            "{cs_label}: only <code> children are declared"
        );
        codes += 1;
        check_attrs(&cs_label, &code, &["value"], &["description", "status"]);
        assert!(
            !code.children().any(|n| n.is_element()),
            "{cs_label}: <code> is empty-content in the XSD"
        );
    }
    assert!(
        codes >= 1,
        "{cs_label}: <code> is maxOccurs=unbounded, min 1"
    );
}

/// One `<group>` element and its `<concept>` children against the XSD.
#[expect(
    clippy::expect_used,
    reason = "a failed lookup after its own presence assertion IS the test failure (Book ch11 assertion idiom)"
)]
fn check_group(label: &str, child: &roxmltree::Node<'_, '_>) {
    let g_label = format!(
        "{label}/group[{}]",
        child.attribute("openehr_id").unwrap_or("?")
    );
    check_attrs(&g_label, child, &["name", "openehr_id"], &["status"]);
    let id = child
        .attribute("openehr_id")
        .expect("checked required above");
    assert!(
        is_ncname(id),
        "{g_label}: @openehr_id={id:?} is not an NCName"
    );
    let mut concepts = 0usize;
    for concept in child.children().filter(roxmltree::Node::is_element) {
        assert_eq!(
            concept.tag_name().name(),
            "concept",
            "{g_label}: only <concept> children are declared"
        );
        concepts += 1;
        check_attrs(&g_label, &concept, &["id", "rubric"], &["status"]);
        let cid = concept.attribute("id").expect("checked required above");
        assert!(
            cid.parse::<i64>().is_ok(),
            "{g_label}: concept@id={cid:?} is not an xs:integer"
        );
        assert!(
            !concept.children().any(|n| n.is_element()),
            "{g_label}: <concept> is empty-content in the XSD"
        );
    }
    assert!(
        concepts >= 1,
        "{g_label}: <concept> is maxOccurs=unbounded, min 1"
    );
}

#[test]
fn language_bundles_conform_to_openehr_terminology_xsd() {
    for (lang, xml) in LANGUAGE_ASSETS {
        let groups_allowed = true;
        let external_id_required = false;
        check_terminology_doc(
            &format!("{lang}/openehr_terminology.xml"),
            xml,
            groups_allowed,
            external_id_required,
        );
    }
}

#[test]
fn external_terminologies_conform_to_their_xsd() {
    let groups_allowed = false;
    let external_id_required = true;
    check_terminology_doc(
        "openehr_external_terminologies.xml",
        EXTERNAL_XML,
        groups_allowed,
        external_id_required,
    );
}

#[test]
fn property_unit_data_conforms_to_property_units_xsd() {
    // assets/schema/PropertyUnitData.xsd: root PropertyUnits in the
    // http://tempuri.org/PropertyUnits.xsd namespace; a sequence of
    // Property* then Unit*; PropertyType requires id (xs:int), Text,
    // openEHR (xs:int); UnitType requires property_id (xs:int) and Text,
    // with optional name, conversion (xs:float), coefficient (xs:int),
    // primary (xs:boolean), UCUM.
    let doc = roxmltree::Document::parse(PROPERTY_UNITS_XML).expect("well-formed XML");
    let root = doc.root_element();
    assert_eq!(root.tag_name().name(), "PropertyUnits");
    assert_eq!(
        root.tag_name().namespace(),
        Some("http://tempuri.org/PropertyUnits.xsd"),
        "the schema's targetNamespace"
    );

    let mut property_ids = std::collections::BTreeSet::new();
    let mut seen_unit = false;
    let mut units = 0usize;
    for child in root.children().filter(roxmltree::Node::is_element) {
        match child.tag_name().name() {
            "Property" => {
                assert!(
                    !seen_unit,
                    "<Property> after <Unit> — the xs:sequence puts every Property first"
                );
                check_attrs("Property", &child, &["id", "Text", "openEHR"], &[]);
                for a in ["id", "openEHR"] {
                    let v = child.attribute(a).expect("checked required above");
                    assert!(
                        v.parse::<i32>().is_ok(),
                        "Property@{a}={v:?} is not an xs:int"
                    );
                }
                property_ids.insert(child.attribute("id").expect("checked").to_owned());
            }
            "Unit" => {
                seen_unit = true;
                units += 1;
                check_attrs(
                    "Unit",
                    &child,
                    &["property_id", "Text"],
                    &["name", "conversion", "coefficient", "primary", "UCUM"],
                );
                let pid = child
                    .attribute("property_id")
                    .expect("checked required above");
                assert!(pid.parse::<i32>().is_ok(), "Unit@property_id is xs:int");
                // Referential sanity on top of the schema: every unit names a
                // declared property (the table is a join, not free text).
                assert!(
                    property_ids.contains(pid),
                    "Unit@property_id={pid:?} names no <Property id=…>"
                );
                if let Some(v) = child.attribute("conversion") {
                    assert!(
                        v.parse::<f32>().is_ok(),
                        "Unit@conversion={v:?} is not an xs:float"
                    );
                }
                if let Some(v) = child.attribute("coefficient") {
                    assert!(
                        v.parse::<i32>().is_ok(),
                        "Unit@coefficient={v:?} is not an xs:int"
                    );
                }
                if let Some(v) = child.attribute("primary") {
                    assert!(
                        matches!(v, "true" | "false" | "1" | "0"),
                        "Unit@primary={v:?} is not an xs:boolean"
                    );
                }
            }
            other => panic!("element <{other}> is not declared by PropertyUnitData.xsd"),
        }
    }
    assert!(!property_ids.is_empty(), "at least one Property row");
    assert!(units >= 1, "at least one Unit row");
}
