// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

#![expect(
    clippy::panic,
    clippy::expect_used,
    reason = "test assertions/diagnostics/fixtures"
)]
//! The OPT 1.4 writer (`opt14_writer`) over the CKM archetype pack and the
//! CKM operational templates.
//!
//! Three gates:
//!
//! 1. Every AM 1.4 archetype the CKM publishes (`corpus/archetypes/ckm/xml/`)
//!    becomes a one-archetype template whose OPT validates against ITS-XML
//!    `Template.xsd`, reads back through the `opt14` reader to the model the
//!    writer built, and serializes to the same bytes twice.
//! 2. Every CKM operational template (`corpus/templates/ckm/full/`, written
//!    by CKM's own OPT generator) is turned back into a template definition
//!    over today's CKM archetypes, and the writer's OPT has the upstream
//!    OPT's node tree: the same archetype roots, nodes, types, occurrences
//!    and existences at the same paths.
//! 3. The writer's refusals, one per typed error.
//!
//! The XSD is the ITS-XML 1.0.2 `ALL/Template.xsd`, which differs from
//! ITS-XML 2.0.0 `AM/Release-1.4/Template.xsd` only in its target namespace;
//! the 2.0.0 bundle does not compile (gate below), so it validates nothing.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
use openehr_base::v1_3::base_types::identification::template_id::TemplateId;
use openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId;
use openehr_base::v1_3::base_types::terminology::code_phrase::CodePhrase;
use openehr_its::opt14::types::{
    Archetype, ArchetypeInternalRef, ArchetypeSlot, Assertion, CArchetypeRoot, CAttribute,
    CMultipleAttribute, CObject, CSingleAttribute, Cardinality, ExprItem, Intervalofinteger,
    OperationalTemplate,
};
use openehr_its::opt14_writer::{
    ArchetypeLibrary, ArchetypeUse, Narrowing, NodeClone, SlotFill, TemplateDefinition, WriteError,
};
use openehr_its::xml::runtime::XmlAny;

use crate::ckm_archetype_xml::{ADJUDICATED, pack_dir, xml_files};

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Every CKM archetype the XML codec reads (the four adjudicated refusals of
/// `ckm_archetype_xml` stay out).
fn library() -> ArchetypeLibrary {
    xml_files(&pack_dir())
        .iter()
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .unwrap_or("");
            !ADJUDICATED.iter().any(|(file, _)| *file == name)
        })
        .map(|path| {
            let xml = std::fs::read_to_string(path).expect("read archetype XML");
            openehr_its::xml::runtime::from_xml::<Archetype>(&xml)
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        })
        .collect()
}

fn archetype<'a>(library: &'a ArchetypeLibrary, id: &str) -> &'a Archetype {
    library
        .get(id)
        .unwrap_or_else(|| panic!("{id} is in the CKM pack"))
}

fn english() -> CodePhrase {
    CodePhrase {
        terminology_id: TerminologyId {
            value: "ISO_639-1".to_owned(),
        },
        code_string: "en".to_owned(),
        preferred_term: None,
    }
}

fn template(id: &str, root: ArchetypeUse) -> TemplateDefinition {
    TemplateDefinition {
        template_id: TemplateId {
            value: id.to_owned(),
        },
        concept: id.to_owned(),
        language: english(),
        description: None,
        uid: None,
        root,
    }
}

fn archetype_use(id: &str) -> ArchetypeUse {
    ArchetypeUse::new(ArchetypeId {
        value: id.to_owned(),
    })
}

fn closed(lower: i32, upper: i32) -> Intervalofinteger {
    Intervalofinteger {
        lower_included: Some(true),
        upper_included: Some(true),
        lower_unbounded: false,
        upper_unbounded: false,
        lower: Some(lower),
        upper: Some(upper),
    }
}

// ---------------------------------------------------------------------------
// XSD validation (xmllint, as the other XSD gates of this crate)
// ---------------------------------------------------------------------------

fn template_xsd() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("schemas/xml/its-xml-1.0.2-nsv1/ALL/Template.xsd")
}

fn scratch(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "openehr-its-opt14-writer-{label}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// Validates every document against `Template.xsd`, returning the
/// diagnostics of each one that fails.
fn xsd_failures(schema: &Path, documents: &[PathBuf]) -> Vec<String> {
    let mut failures = Vec::new();
    for chunk in documents.chunks(64) {
        let out = Command::new("xmllint")
            .arg("--noout")
            .arg("--schema")
            .arg(schema)
            .args(chunk)
            .output()
            .expect("run xmllint (the XSD gates need it)");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("failed to compile"),
            "{} does not compile: {stderr}",
            schema.display()
        );
        for document in chunk {
            let name = document.display().to_string();
            if !stderr.contains(&format!("{name} validates")) {
                let lines: Vec<&str> = stderr.lines().filter(|l| l.starts_with(&name)).collect();
                failures.push(lines.join("\n"));
            }
        }
    }
    failures
}

// ---------------------------------------------------------------------------
// Node tree: the comparison key of gate 2
// ---------------------------------------------------------------------------

fn kind(object: &CObject) -> &'static str {
    match object {
        CObject::ArchetypeInternalRef(_) => "ARCHETYPE_INTERNAL_REF",
        CObject::ArchetypeSlot(_) => "ARCHETYPE_SLOT",
        CObject::ConstraintRef(_) => "CONSTRAINT_REF",
        CObject::CArchetypeRoot(_) => "C_ARCHETYPE_ROOT",
        CObject::CCodePhrase(_) => "C_CODE_PHRASE",
        CObject::CCodeReference(_) => "C_CODE_REFERENCE",
        CObject::CComplexObject(_) => "C_COMPLEX_OBJECT",
        CObject::CDefinedObject(_) => "C_DEFINED_OBJECT",
        CObject::CDvOrdinal(_) => "C_DV_ORDINAL",
        CObject::CDvQuantity(_) => "C_DV_QUANTITY",
        CObject::CDvState(_) => "C_DV_STATE",
        CObject::CPrimitiveObject(_) => "C_PRIMITIVE_OBJECT",
        CObject::TComplexObject(_) => "T_COMPLEX_OBJECT",
    }
}

/// `(rm_type_name, occurrences, node_id)` of any `C_OBJECT`.
fn head(object: &CObject) -> (&str, &Intervalofinteger, &str) {
    match object {
        CObject::ArchetypeInternalRef(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::ArchetypeSlot(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::ConstraintRef(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CArchetypeRoot(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CCodePhrase(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CCodeReference(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CComplexObject(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CDefinedObject(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CDvOrdinal(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CDvQuantity(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CDvState(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::CPrimitiveObject(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
        CObject::TComplexObject(o) => (&o.rm_type_name, &o.occurrences, &o.node_id),
    }
}

fn own_attributes(object: &CObject) -> &[CAttribute] {
    match object {
        CObject::CComplexObject(o) => &o.attributes,
        CObject::TComplexObject(o) => &o.attributes,
        CObject::CArchetypeRoot(o) => &o.attributes,
        _ => &[],
    }
}

fn attribute_parts(attribute: &CAttribute) -> (&str, &Intervalofinteger, &[CObject]) {
    match attribute {
        CAttribute::CMultipleAttribute(a) => (&a.rm_attribute_name, &a.existence, &a.children),
        CAttribute::CSingleAttribute(a) => (&a.rm_attribute_name, &a.existence, &a.children),
    }
}

fn interval_text(i: &Intervalofinteger) -> String {
    let lower = if i.lower_unbounded {
        "*".to_owned()
    } else {
        i.lower.map_or("?".to_owned(), |v| v.to_string())
    };
    let upper = if i.upper_unbounded {
        "*".to_owned()
    } else {
        i.upper.map_or("?".to_owned(), |v| v.to_string())
    };
    format!("{lower}..{upper}")
}

/// The node tree of an OPT: each path with its sorted node and attribute
/// facts. An archetype root's predicate is its archetype id, as in an OPT 1.4
/// path; sibling order is not compared.
fn node_tree(root: &CArchetypeRoot) -> BTreeMap<String, Vec<String>> {
    let mut tree = BTreeMap::new();
    let path = format!("[{}]", root.archetype_id.value);
    tree.insert(
        path.clone(),
        vec![format!(
            "C_ARCHETYPE_ROOT {} {}",
            root.rm_type_name,
            interval_text(&root.occurrences)
        )],
    );
    walk_attributes(&root.attributes, &path, &mut tree);
    tree
}

fn walk_attributes(
    attributes: &[CAttribute],
    path: &str,
    tree: &mut BTreeMap<String, Vec<String>>,
) {
    for attribute in attributes {
        let (name, existence, children) = attribute_parts(attribute);
        let attribute_path = format!("{path}/{name}");
        let facts = tree.entry(attribute_path.clone()).or_default();
        facts.push(format!("existence {}", interval_text(existence)));
        facts.sort();
        for child in children {
            let (rm_type, occurrences, node_id) = head(child);
            let child_path = match child {
                CObject::CArchetypeRoot(r) => format!("{attribute_path}[{}]", r.archetype_id.value),
                _ if node_id.is_empty() => attribute_path.clone(),
                _ => format!("{attribute_path}[{node_id}]"),
            };
            let facts = tree.entry(child_path.clone()).or_default();
            facts.push(format!(
                "{} {rm_type} {}",
                kind(child),
                interval_text(occurrences)
            ));
            facts.sort();
            walk_attributes(own_attributes(child), &child_path, tree);
        }
    }
}

fn contains_internal_ref(attributes: &[CAttribute]) -> bool {
    attributes.iter().any(|a| {
        attribute_parts(a).2.iter().any(|child| {
            matches!(child, CObject::ArchetypeInternalRef(_))
                || contains_internal_ref(own_attributes(child))
        })
    })
}

// ---------------------------------------------------------------------------
// Gate 1: every CKM archetype as a one-archetype template
// ---------------------------------------------------------------------------

/// Archetypes whose OPT fails `Template.xsd` because the archetype itself
/// does: the writer copies an archetype's constraints as they are. Each entry
/// is a negative test, so a corrected CKM export shows up here.
const XSD_INVALID_ARCHETYPES: &[(&str, &str)] = &[(
    "openEHR-EHR-OBSERVATION.age_assertion.v1",
    "a C_DURATION range bound `-P1Y` breaks the `Iso8601Duration` pattern of \
     its-xml-1.0.2-nsv1 BaseTypes.xsd, in the CKM archetype XML as in the OPT",
)];

#[test]
fn every_ckm_archetype_writes_an_opt_that_validates_and_reads_back() {
    let library = library();
    assert!(library.len() >= 900, "the CKM archetype pack is missing");
    let dir = scratch("single");
    let mut documents = Vec::new();
    let mut adjudicated = Vec::new();
    let mut findings = String::new();
    for (index, id) in xml_files(&pack_dir())
        .iter()
        .filter_map(|p| p.file_stem().and_then(std::ffi::OsStr::to_str))
        .filter(|id| library.get(id).is_some())
        .enumerate()
    {
        let mut definition = template(id, archetype_use(id));
        definition.language = archetype(&library, id).original_language.clone();
        let built = match openehr_its::opt14_writer::build(&definition, &library) {
            Ok(built) => built,
            Err(e) => {
                let _ = writeln!(findings, "{id}: {e}");
                continue;
            }
        };
        let xml = openehr_its::opt14_writer::write(&definition, &library).expect("writes");
        assert_eq!(
            openehr_its::opt14_writer::write(&definition, &library).expect("writes"),
            xml,
            "{id}: the same inputs give the same bytes"
        );
        let read = openehr_its::opt14::from_xml(&xml)
            .unwrap_or_else(|e| panic!("{id}: the opt14 reader refuses the writer's OPT: {e}"));
        assert_eq!(
            read, built,
            "{id}: the OPT reads back to the model the writer built"
        );
        assert!(
            !contains_internal_ref(&built.definition.attributes),
            "{id}: every internal reference is inlined"
        );
        let document = dir.join(format!("{index:04}.opt"));
        std::fs::write(&document, xml).expect("write OPT");
        if XSD_INVALID_ARCHETYPES.iter().any(|(bad, _)| *bad == id) {
            adjudicated.push((id.to_owned(), document));
        } else {
            documents.push(document);
        }
    }
    assert!(
        findings.is_empty(),
        "archetypes the writer refuses:\n{findings}"
    );
    assert_eq!(
        adjudicated.len(),
        XSD_INVALID_ARCHETYPES.len(),
        "every adjudicated archetype is in the pack"
    );
    for (id, document) in &adjudicated {
        let source = pack_dir().join(format!("{id}.xml"));
        assert_eq!(
            xsd_failures(&template_xsd(), &[source, document.clone()]).len(),
            2,
            "{id} is adjudicated XSD-invalid in the archetype and its OPT, and one now validates"
        );
    }
    let failures = xsd_failures(&template_xsd(), &documents);
    std::fs::remove_dir_all(&dir).expect("remove scratch dir");
    assert!(
        failures.is_empty(),
        "{} of {} OPTs fail Template.xsd:\n{}",
        failures.len(),
        documents.len(),
        failures.join("\n")
    );
}

#[test]
fn the_library_order_does_not_change_the_bytes() {
    let library = library();
    let reversed: ArchetypeLibrary = xml_files(&pack_dir())
        .iter()
        .rev()
        .filter_map(|p| p.file_stem().and_then(std::ffi::OsStr::to_str))
        .filter_map(|id| library.get(id).cloned())
        .collect();
    let definition = birth_summary_with_fills();
    assert_eq!(
        openehr_its::opt14_writer::write(&definition, &library).expect("writes"),
        openehr_its::opt14_writer::write(&definition, &reversed).expect("writes"),
    );
}

/// The ITS-XML 2.0.0 OPT 1.4 bundle declares `VALIDITY_KIND` twice
/// (`AM/Release-1.4/Archetype.xsd` and `RM/Release-1.0.2/BaseTypes.xsd`, one
/// target namespace), so no XSD processor compiles it. Pinned so a fixed
/// re-vendor shows up here and gate 1 can validate against it too.
#[test]
fn the_its_xml_2_0_0_release_1_4_bundle_does_not_compile() {
    let schema = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas/xml/its-xml-2.0.0-nsv2/AM/Release-1.4/Template.xsd");
    let dir = scratch("v2-probe");
    let document = dir.join("probe.opt");
    std::fs::write(
        &document,
        "<template xmlns=\"http://schemas.openehr.org/v2\"/>\n",
    )
    .expect("write probe");
    let out = Command::new("xmllint")
        .arg("--noout")
        .arg("--schema")
        .arg(&schema)
        .arg(&document)
        .output()
        .expect("run xmllint");
    std::fs::remove_dir_all(&dir).expect("remove scratch dir");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to compile") && stderr.contains("VALIDITY_KIND"),
        "the 2.0.0 Release-1.4 bundle now compiles; validate gate 1 against it: {stderr}"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: the CKM operational templates, rebuilt
// ---------------------------------------------------------------------------

/// Why a CKM template has no definition over today's archetypes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Drift {
    /// It names an archetype version the CKM pack no longer publishes.
    ArchetypeVersion,
    /// The archetype was revised: a node or attribute the template uses is
    /// gone, or a mandatory one was added.
    ArchetypeRevised,
    /// The template repeats a node in a way no clone admits: a node that
    /// occurs at most once, a copy without a `name` constraint, or two copies
    /// with the same one.
    InvalidClone,
    /// The OPT names a primitive object's type as the AOM primitive
    /// (`DURATION`) where the CKM archetype XML names the data value type
    /// (`DV_DURATION`), so neither conforms to the other.
    PrimitiveTypeName,
    /// The OPT gives a node a type that does not conform to the archetype's
    /// (`DV_DATE` where the archetype has `DV_DATE_TIME`): a revised
    /// archetype, or a redefinition the specification does not admit.
    NonConformingType,
    /// The archetype has several slots that admit a filler, and the OPT does
    /// not say which one it fills.
    AmbiguousSlot,
    /// The template constrains something a template definition does not
    /// express.
    Inexpressible,
}

fn ckm_templates_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates/ckm/full")
}

/// Turns a CKM OPT back into a template definition over `library`.
fn derive(
    opt: &OperationalTemplate,
    library: &ArchetypeLibrary,
) -> Result<TemplateDefinition, Drift> {
    let root = derive_use(&opt.definition, library)?;
    Ok(TemplateDefinition {
        template_id: opt.template_id.clone(),
        concept: opt.concept.clone(),
        language: opt.language.clone(),
        description: opt.description.clone(),
        uid: opt.uid.clone(),
        root,
    })
}

fn derive_use(root: &CArchetypeRoot, library: &ArchetypeLibrary) -> Result<ArchetypeUse, Drift> {
    let id = root.archetype_id.value.as_str();
    let archetype = library.get(id).ok_or(Drift::ArchetypeVersion)?;
    let mut archetype_use = archetype_use(id);
    derive_attributes(
        &root.attributes,
        &archetype.definition.attributes,
        "",
        library,
        &mut archetype_use,
    )?;
    Ok(archetype_use)
}

fn is_mandatory(i: &Intervalofinteger) -> bool {
    !i.lower_unbounded && i.lower.is_some_and(|l| l >= 1)
}

/// Whether every value of `inner` lies in `outer` (closed bounds, as the CKM
/// OPTs write them).
fn within(inner: &Intervalofinteger, outer: &Intervalofinteger) -> bool {
    let lower = outer.lower_unbounded
        || (!inner.lower_unbounded && inner.lower.unwrap_or(0) >= outer.lower.unwrap_or(0));
    let upper = outer.upper_unbounded
        || (!inner.upper_unbounded && inner.upper.unwrap_or(0) <= outer.upper.unwrap_or(0));
    lower && upper
}

fn same_interval(a: &Intervalofinteger, b: &Intervalofinteger) -> bool {
    interval_text(a) == interval_text(b)
}

fn derive_attributes(
    opt: &[CAttribute],
    archetype: &[CAttribute],
    path: &str,
    library: &ArchetypeLibrary,
    out: &mut ArchetypeUse,
) -> Result<(), Drift> {
    let owner = if path.is_empty() { "/" } else { path };
    for opt_attribute in opt {
        let (name, existence, children) = attribute_parts(opt_attribute);
        if archetype.iter().any(|a| attribute_parts(a).0 == name) {
            continue;
        }
        let roots: Vec<&CArchetypeRoot> = children
            .iter()
            .filter_map(|c| match c {
                CObject::CArchetypeRoot(r) => Some(r),
                _ => None,
            })
            .collect();
        let attribute_path = format!("{path}/{name}");
        if roots.len() == children.len() && !roots.is_empty() {
            out.narrowings.push(Narrowing::Existence {
                path: attribute_path.clone(),
                existence: existence.clone(),
            });
        } else {
            let mut constraint = opt_attribute.clone();
            children_mut(&mut constraint).retain(|c| !matches!(c, CObject::CArchetypeRoot(_)));
            out.narrowings.push(Narrowing::Constrain {
                path: owner.to_owned(),
                attribute: constraint,
            });
        }
        for root in roots {
            out.fills.push(SlotFill {
                path: attribute_path.clone(),
                occurrences: Some(root.occurrences.clone()),
                filler: derive_use(root, library)?,
            });
        }
    }
    for arch_attribute in archetype {
        let (name, arch_existence, arch_children) = attribute_parts(arch_attribute);
        let attribute_path = format!("{path}/{name}");
        let Some(opt_attribute) = opt.iter().find(|a| attribute_parts(a).0 == name) else {
            if is_mandatory(arch_existence) {
                return Err(Drift::ArchetypeRevised);
            }
            out.narrowings.push(Narrowing::Existence {
                path: attribute_path,
                existence: closed(0, 0),
            });
            continue;
        };
        let (_, opt_existence, opt_children) = attribute_parts(opt_attribute);
        if !same_interval(opt_existence, arch_existence) {
            out.narrowings.push(Narrowing::Existence {
                path: attribute_path.clone(),
                existence: opt_existence.clone(),
            });
        }
        let multiple = matches!(arch_attribute, CAttribute::CMultipleAttribute(_));
        derive_children(
            opt_children,
            arch_children,
            &attribute_path,
            multiple,
            library,
            out,
        )?;
    }
    Ok(())
}

fn children_mut(attribute: &mut CAttribute) -> &mut Vec<CObject> {
    match attribute {
        CAttribute::CMultipleAttribute(a) => &mut a.children,
        CAttribute::CSingleAttribute(a) => &mut a.children,
    }
}

fn is_root(object: &CObject) -> bool {
    matches!(object, CObject::CArchetypeRoot(_))
}

/// The subtree facts of one object, the node-tree comparison key below it.
fn subtree(object: &CObject) -> BTreeMap<String, Vec<String>> {
    let mut tree = BTreeMap::new();
    let (rm_type, occurrences, _) = head(object);
    tree.insert(
        String::new(),
        vec![format!(
            "{} {rm_type} {}",
            kind(object),
            interval_text(occurrences)
        )],
    );
    walk_attributes(own_attributes(object), "", &mut tree);
    tree
}

/// The archetype alternative (a child without node id) an OPT object without
/// node id redefines: the one of its exact type, else the only one its type
/// conforms to.
fn matched_alternative<'a>(object: &CObject, alternatives: &[&'a CObject]) -> Option<&'a CObject> {
    let rm_type = head(object).0;
    let exact: Vec<&&CObject> = alternatives
        .iter()
        .filter(|a| head(a).0 == rm_type && kind(a) == kind(object))
        .collect();
    if let [only] = exact.as_slice() {
        return Some(only);
    }
    let class = |t: &str| t.split('<').next().unwrap_or(t).to_owned();
    let conforming: Vec<&&CObject> = alternatives
        .iter()
        .filter(|a| {
            let archetype_type = head(a).0;
            rm_type == archetype_type
                || (openehr_rm::v1_2::model::is_a(&class(rm_type), &class(archetype_type))
                    && (!archetype_type.contains('<')
                        || rm_type.split_once('<').map(|p| p.1)
                            == archetype_type.split_once('<').map(|p| p.1)))
        })
        .collect();
    match conforming.as_slice() {
        [only] => Some(only),
        _ => None,
    }
}

/// Why an OPT object without node id redefines none of the archetype's
/// alternatives.
fn unmatched(object: &CObject, alternatives: &[&CObject]) -> Drift {
    let primitive = |o: &CObject| matches!(o, CObject::CPrimitiveObject(_));
    if primitive(object) && alternatives.iter().any(|a| primitive(a)) {
        Drift::PrimitiveTypeName
    } else if alternatives.is_empty() {
        Drift::Inexpressible
    } else {
        Drift::NonConformingType
    }
}

/// An OPT object as a redefinition of the archetype object `archetype`: the
/// attributes the archetype constrains and the OPT drops at existence `{0}`,
/// the archetype's named children the OPT drops at occurrences `{0}`.
fn differential(opt: &CObject, archetype: &CObject) -> Result<CObject, Drift> {
    let arch_attributes = own_attributes(archetype);
    let mut out = opt.clone();
    let opt_attributes = own_attributes(opt).to_vec();
    let Some(attributes) = (match &mut out {
        CObject::CComplexObject(o) => Some(&mut o.attributes),
        _ => None,
    }) else {
        return Ok(out);
    };
    if !matches!(archetype, CObject::CComplexObject(_)) {
        return Ok(out);
    }
    attributes.clear();
    for opt_attribute in &opt_attributes {
        let name = attribute_parts(opt_attribute).0;
        match arch_attributes
            .iter()
            .find(|a| attribute_parts(a).0 == name)
        {
            None => attributes.push(opt_attribute.clone()),
            Some(arch_attribute) => {
                attributes.push(differential_attribute(opt_attribute, arch_attribute)?);
            }
        }
    }
    for arch_attribute in arch_attributes {
        let name = attribute_parts(arch_attribute).0;
        if opt_attributes.iter().all(|a| attribute_parts(a).0 != name) {
            if is_mandatory(attribute_parts(arch_attribute).1) {
                return Err(Drift::ArchetypeRevised);
            }
            let mut removed = arch_attribute.clone();
            children_mut(&mut removed).clear();
            match &mut removed {
                CAttribute::CMultipleAttribute(a) => a.existence = closed(0, 0),
                CAttribute::CSingleAttribute(a) => a.existence = closed(0, 0),
            }
            attributes.push(removed);
        }
    }
    Ok(out)
}

fn differential_attribute(opt: &CAttribute, archetype: &CAttribute) -> Result<CAttribute, Drift> {
    let (_, _, opt_children) = attribute_parts(opt);
    let (_, _, arch_children) = attribute_parts(archetype);
    let mut out = opt.clone();
    let children = children_mut(&mut out);
    children.clear();
    let alternatives: Vec<&CObject> = arch_children
        .iter()
        .filter(|c| !is_root(c) && head(c).2.is_empty())
        .collect();
    for opt_child in opt_children {
        if is_root(opt_child) {
            return Err(Drift::Inexpressible);
        }
        let node_id = head(opt_child).2;
        let arch_child = if node_id.is_empty() {
            matched_alternative(opt_child, &alternatives)
                .ok_or_else(|| unmatched(opt_child, &alternatives))?
        } else {
            if opt_children.iter().filter(|c| head(c).2 == node_id).count() > 1 {
                return Err(Drift::Inexpressible);
            }
            arch_children
                .iter()
                .find(|c| !is_root(c) && head(c).2 == node_id)
                .ok_or(Drift::ArchetypeRevised)?
        };
        children.push(differential(opt_child, arch_child)?);
    }
    let restated = opt_children.iter().any(|c| head(c).2.is_empty());
    if !alternatives.is_empty() && !restated && !opt_children.is_empty() {
        return Err(Drift::Inexpressible);
    }
    for arch_child in arch_children.iter().filter(|c| !is_root(c)) {
        let node_id = head(arch_child).2;
        if node_id.is_empty() || opt_children.iter().any(|c| head(c).2 == node_id) {
            continue;
        }
        if is_mandatory(head(arch_child).1) {
            return Err(Drift::ArchetypeRevised);
        }
        let mut excluded = arch_child.clone();
        match &mut excluded {
            CObject::CComplexObject(o) => o.occurrences = closed(0, 0),
            _ => return Err(Drift::Inexpressible),
        }
        children.push(excluded);
    }
    Ok(out)
}

/// The `name` attribute of a complex object, if it constrains one.
fn name_attribute(object: &CObject) -> Option<&CAttribute> {
    own_attributes(object)
        .iter()
        .find(|a| attribute_parts(a).0 == "name")
}

/// A repeated archetype node as clones: the first OPT copy is the node
/// itself, every later one a [`NodeClone`] with its narrowings relative to
/// the copy.
fn derive_clones(
    copies: &[&CObject],
    archetype: &CObject,
    path: &str,
    single_valued: bool,
    library: &ArchetypeLibrary,
    out: &mut ArchetypeUse,
) -> Result<(), Drift> {
    let (_, arch_occurrences, _) = head(archetype);
    let at_most_once = !arch_occurrences.upper_unbounded && arch_occurrences.upper <= Some(1);
    if single_valued || at_most_once {
        return Err(Drift::InvalidClone);
    }
    if name_attribute(archetype).is_some() {
        return Err(Drift::Inexpressible);
    }
    let names: Vec<&CAttribute> = copies.iter().filter_map(|c| name_attribute(c)).collect();
    let named_later = copies.iter().skip(1).all(|c| name_attribute(c).is_some());
    let distinct = names
        .iter()
        .enumerate()
        .all(|(i, n)| names.iter().skip(i + 1).all(|m| m != n));
    if !named_later || !distinct {
        return Err(Drift::InvalidClone);
    }
    for copy in copies.iter().skip(1) {
        let name = name_attribute(copy).ok_or(Drift::InvalidClone)?.clone();
        let mut inner = archetype_use("unused");
        let attributes: Vec<CAttribute> = own_attributes(copy)
            .iter()
            .filter(|a| attribute_parts(a).0 != "name")
            .cloned()
            .collect();
        derive_attributes(
            &attributes,
            own_attributes(archetype),
            "",
            library,
            &mut inner,
        )?;
        let (_, occurrences, _) = head(copy);
        let mut clone = NodeClone::new(path, name);
        clone.occurrences =
            (!same_interval(occurrences, arch_occurrences)).then(|| occurrences.clone());
        clone.fills = inner.fills;
        clone.narrowings = inner.narrowings;
        out.narrowings.push(Narrowing::Clone(clone));
    }
    Ok(())
}

/// The archetype's children without node id against the OPT's, as one
/// [`Narrowing::Redefine`] when the template retypes, drops or (where no
/// path addresses them) changes them; `None` when the path-wise derivation
/// below expresses them.
fn derive_alternatives(
    opt: &[&CObject],
    alternatives: &[&CObject],
    attribute_path: &str,
) -> Result<Option<Narrowing>, Drift> {
    let mut objects = Vec::new();
    let mut changed = false;
    let mut matched = vec![false; alternatives.len()];
    for opt_child in opt {
        let Some(arch_child) = matched_alternative(opt_child, alternatives) else {
            return Err(unmatched(opt_child, alternatives));
        };
        let index = alternatives
            .iter()
            .position(|a| std::ptr::eq(*a, arch_child));
        if let Some(seen) = index.and_then(|i| matched.get_mut(i)) {
            changed |= *seen;
            *seen = true;
        }
        changed |= head(opt_child).0 != head(arch_child).0
            || (alternatives.len() > 1 && subtree(opt_child) != subtree(arch_child));
        objects.push(differential(opt_child, arch_child)?);
    }
    changed |= matched.iter().any(|m| !m);
    Ok(changed.then(|| Narrowing::Redefine {
        path: attribute_path.to_owned(),
        objects,
    }))
}

fn derive_children(
    opt: &[CObject],
    sources: &[CObject],
    attribute_path: &str,
    multiple: bool,
    library: &ArchetypeLibrary,
    out: &mut ArchetypeUse,
) -> Result<(), Drift> {
    let own: Vec<&CObject> = opt.iter().filter(|c| !is_root(c)).collect();
    let alternatives: Vec<&CObject> = sources
        .iter()
        .filter(|c| !is_root(c) && head(c).2.is_empty())
        .collect();
    let opt_alternatives: Vec<&CObject> = own
        .iter()
        .copied()
        .filter(|c| head(c).2.is_empty())
        .collect();
    for opt_child in &own {
        let (_, _, node_id) = head(opt_child);
        if node_id.is_empty() {
            continue;
        }
        if !sources.iter().any(|a| head(a).2 == node_id) {
            return Err(Drift::ArchetypeRevised);
        }
    }
    let redefined = if alternatives.is_empty() {
        if !opt_alternatives.is_empty() {
            return Err(Drift::ArchetypeRevised);
        }
        false
    } else if opt_alternatives.is_empty() {
        if alternatives.iter().any(|a| is_mandatory(head(a).1)) && alternatives.len() == 1 {
            return Err(Drift::ArchetypeRevised);
        }
        if alternatives.len() > 1 {
            return Err(Drift::Inexpressible);
        }
        false
    } else if let Some(narrowing) =
        derive_alternatives(&opt_alternatives, &alternatives, attribute_path)?
    {
        out.narrowings.push(narrowing);
        true
    } else {
        false
    };
    for arch_child in sources.iter().filter(|c| !is_root(c)) {
        let (_, arch_occurrences, arch_id) = head(arch_child);
        if arch_id.is_empty() && (redefined || alternatives.len() > 1) {
            continue;
        }
        let path = if arch_id.is_empty() {
            attribute_path.to_owned()
        } else {
            format!("{attribute_path}[{arch_id}]")
        };
        let copies: Vec<&CObject> = own
            .iter()
            .copied()
            .filter(|o| {
                if arch_id.is_empty() {
                    head(o).2.is_empty() && head(o).0 == head(arch_child).0
                } else {
                    head(o).2 == arch_id
                }
            })
            .collect();
        let Some(counterpart) = copies.first() else {
            if is_mandatory(arch_occurrences) {
                return Err(Drift::ArchetypeRevised);
            }
            out.narrowings.push(Narrowing::Exclude { path });
            continue;
        };
        if copies.len() > 1 {
            derive_clones(&copies, arch_child, &path, !multiple, library, out)?;
        }
        let (_, opt_occurrences, _) = head(counterpart);
        if !same_interval(opt_occurrences, arch_occurrences) {
            out.narrowings.push(Narrowing::Occurrences {
                path: path.clone(),
                occurrences: opt_occurrences.clone(),
            });
        }
        let opt_inner = own_attributes(counterpart);
        let arch_inner = own_attributes(arch_child);
        if !arch_inner.is_empty() || !opt_inner.is_empty() {
            derive_attributes(opt_inner, arch_inner, &path, library, out)?;
        }
    }
    for filler in opt.iter().filter_map(|c| match c {
        CObject::CArchetypeRoot(r) => Some(r),
        _ => None,
    }) {
        let slot_path = choose_slot(sources, opt, attribute_path, filler)?;
        out.fills.push(SlotFill {
            path: slot_path,
            occurrences: Some(filler.occurrences.clone()),
            filler: derive_use(filler, library)?,
        });
    }
    Ok(())
}

/// The slot a filler sits in: the slot whose include pattern names it, else
/// the only slot that admits it, else the slot the OPT places it after, else
/// the first slot whose occurrences admit the filler's. An OPT 1.4 does not
/// record a filler's slot, so the last choice changes only sibling order.
fn choose_slot(
    archetype: &[CObject],
    opt: &[CObject],
    attribute_path: &str,
    filler: &CArchetypeRoot,
) -> Result<String, Drift> {
    let id = filler.archetype_id.value.as_str();
    let admits = |slot: &&ArchetypeSlot, specific: bool| {
        slot.includes.iter().any(|assertion| {
            pattern_of(assertion).is_some_and(|p| {
                regex::Regex::new(&format!("^(?:{p})$")).is_ok_and(|r| r.is_match(id))
                    && (!specific || !matches!(p.as_str(), ".*" | ".+"))
            })
        })
    };
    let slots: Vec<&ArchetypeSlot> = archetype
        .iter()
        .filter_map(|c| match c {
            CObject::ArchetypeSlot(s) => Some(s),
            _ => None,
        })
        .collect();
    let named: Vec<&&ArchetypeSlot> = slots.iter().filter(|s| admits(s, true)).collect();
    let open: Vec<&&ArchetypeSlot> = slots
        .iter()
        .filter(|s| admits(s, false) || s.includes.is_empty())
        .collect();
    let preceding = || {
        let at = opt
            .iter()
            .position(|c| matches!(c, CObject::CArchetypeRoot(r) if std::ptr::eq(r, filler)))?;
        opt.iter().take(at).rev().find_map(|c| match c {
            CObject::ArchetypeSlot(s) if open.iter().any(|o| o.node_id == s.node_id) => {
                Some(s.node_id.clone())
            }
            _ => None,
        })
    };
    let node_id = match (named.as_slice(), open.as_slice()) {
        ([only], _) => only.node_id.clone(),
        (_, [only]) if slots.len() == 1 || named.is_empty() => only.node_id.clone(),
        _ if slots.is_empty() => return Err(Drift::ArchetypeRevised),
        _ => preceding()
            .or_else(|| {
                open.iter()
                    .find(|s| within(&filler.occurrences, &s.occurrences))
                    .map(|s| s.node_id.clone())
            })
            .ok_or(Drift::AmbiguousSlot)?,
    };
    Ok(format!("{attribute_path}[{node_id}]"))
}

/// The `<pattern>` of an `archetype_id/value matches {/pattern/}` assertion.
fn pattern_of(assertion: &Assertion) -> Option<String> {
    let ExprItem::ExprBinaryOperator(op) = assertion.expression.as_ref() else {
        return None;
    };
    let ExprItem::ExprLeaf(right) = op.right_operand.as_ref() else {
        return None;
    };
    right.item.child("pattern").map(XmlAny::text)
}

#[test]
fn ckm_templates_rebuild_to_their_upstream_node_tree() {
    let library = library();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(ckm_templates_dir())
        .expect("read the CKM template pack")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "opt"))
        .collect();
    paths.sort();
    assert!(paths.len() >= 300, "the CKM template pack is missing");

    let dir = scratch("ckm");
    let mut drift: BTreeMap<Drift, usize> = BTreeMap::new();
    let mut findings = String::new();
    let mut documents = Vec::new();
    for path in &paths {
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or("?");
        let xml = std::fs::read_to_string(path).expect("read OPT");
        let Ok(upstream) = openehr_its::opt14::from_xml(&xml) else {
            continue;
        };
        let definition = match derive(&upstream, &library) {
            Ok(definition) => definition,
            Err(d) => {
                *drift.entry(d).or_default() += 1;
                continue;
            }
        };
        let built = match openehr_its::opt14_writer::build(&definition, &library) {
            Ok(built) => built,
            Err(e) => {
                let _ = writeln!(
                    findings,
                    "{name}: the writer refuses the derived definition: {e}"
                );
                continue;
            }
        };
        let ours = node_tree(&built.definition);
        let theirs = node_tree(&upstream.definition);
        if ours != theirs {
            let mut diff = String::new();
            for (key, facts) in &theirs {
                if ours.get(key) != Some(facts) {
                    let _ = writeln!(
                        diff,
                        "  {key}: upstream {facts:?}, ours {:?}",
                        ours.get(key)
                    );
                }
            }
            for key in ours.keys().filter(|k| !theirs.contains_key(*k)) {
                let _ = writeln!(diff, "  {key}: only ours {:?}", ours.get(key));
            }
            let _ = writeln!(findings, "{name}: node trees differ:\n{diff}");
            continue;
        }
        assert_eq!(built.template_id, upstream.template_id, "{name}");
        assert_eq!(built.description, upstream.description, "{name}");
        let written = openehr_its::opt14::to_xml(&built).expect("serializes");
        assert_eq!(
            openehr_its::opt14::from_xml(&written).expect("reads back"),
            built,
            "{name}: the OPT reads back to the model the writer built"
        );
        let document = dir.join(name);
        std::fs::write(&document, written).expect("write OPT");
        documents.push(document);
    }
    let failures = xsd_failures(&template_xsd(), &documents);
    std::fs::remove_dir_all(&dir).expect("remove scratch dir");
    println!(
        "CKM templates: {} files, {} rebuilt to the upstream node tree, drift {drift:?}",
        paths.len(),
        documents.len()
    );
    assert!(
        findings.is_empty(),
        "CKM templates the writer does not rebuild:\n{findings}"
    );
    assert!(
        failures.is_empty(),
        "rebuilt OPTs that fail Template.xsd:\n{}",
        failures.join("\n")
    );
    assert!(
        documents.len() >= 140,
        "only {} CKM templates rebuilt; the gate is near vacuous",
        documents.len()
    );
}

// ---------------------------------------------------------------------------
// Gate 3: shapes and refusals
// ---------------------------------------------------------------------------

const BIRTH_SUMMARY: &str = "openEHR-EHR-EVALUATION.birth_summary.v0";
const ADDRESS: &str = "openEHR-EHR-CLUSTER.address.v1";
const DEVICE: &str = "openEHR-EHR-CLUSTER.device.v1";
const PLACE_OF_BIRTH_SLOT: &str = "/data[at0001]/items[at0005]";

fn birth_summary_with_fills() -> TemplateDefinition {
    let mut root = archetype_use(BIRTH_SUMMARY);
    root.fills.push(SlotFill {
        path: PLACE_OF_BIRTH_SLOT.to_owned(),
        occurrences: Some(closed(0, 1)),
        filler: archetype_use(ADDRESS),
    });
    template("Birth summary", root)
}

fn items_children(template: &OperationalTemplate) -> &[CObject] {
    let data = template
        .definition
        .attributes
        .iter()
        .find(|a| attribute_parts(a).0 == "data")
        .expect("data");
    let tree = attribute_parts(data).2.first().expect("the ITEM_TREE");
    let items = own_attributes(tree)
        .iter()
        .find(|a| attribute_parts(a).0 == "items")
        .expect("items");
    attribute_parts(items).2
}

#[test]
fn a_filler_follows_its_slot_as_an_archetype_root() {
    let library = library();
    let built =
        openehr_its::opt14_writer::build(&birth_summary_with_fills(), &library).expect("builds");
    assert_eq!(
        built
            .definition
            .template_id
            .as_ref()
            .map(|t| t.value.as_str()),
        Some("Birth summary")
    );
    assert_eq!(built.definition.node_id, "at0000");
    let children = items_children(&built);
    let slot = children
        .iter()
        .position(|c| head(c).2 == "at0005")
        .expect("the slot stays open");
    let CObject::CArchetypeRoot(root) = &children[slot + 1] else {
        panic!("the filler follows its slot");
    };
    assert_eq!(root.archetype_id.value, ADDRESS);
    assert_eq!(root.node_id, "at0000");
    assert_eq!(root.occurrences, closed(0, 1));
    assert!(root.template_id.is_none());
    let address = archetype(&library, ADDRESS);
    let english_terms = address
        .ontology
        .term_definitions
        .iter()
        .find(|set| set.language == "en")
        .expect("English terms");
    assert_eq!(root.term_definitions, english_terms.items);
    let components: Vec<&str> = built
        .component_ontologies
        .iter()
        .map(|o| o.archetype_id.as_str())
        .collect();
    assert_eq!(components, [BIRTH_SUMMARY, ADDRESS]);
    assert_eq!(
        built.component_ontologies[1].term_definitions,
        address.ontology.term_definitions
    );
}

#[test]
fn excluding_a_filled_slot_closes_it() {
    let library = library();
    let mut definition = birth_summary_with_fills();
    definition.root.narrowings.push(Narrowing::Exclude {
        path: PLACE_OF_BIRTH_SLOT.to_owned(),
    });
    let built = openehr_its::opt14_writer::build(&definition, &library).expect("builds");
    let children = items_children(&built);
    assert!(
        !children.iter().any(|c| head(c).2 == "at0005"),
        "the slot is closed"
    );
    assert!(
        children
            .iter()
            .any(|c| matches!(c, CObject::CArchetypeRoot(r) if r.archetype_id.value == ADDRESS)),
        "the filler stays"
    );
}

#[test]
fn a_lone_include_admits_any_filler_of_the_slot_type() {
    let library = library();
    let mut definition = birth_summary_with_fills();
    definition.root.fills[0].filler = archetype_use(DEVICE);
    assert!(openehr_its::opt14_writer::build(&definition, &library).is_ok());
}

#[test]
fn a_binding_slot_refuses_a_filler_it_does_not_name() {
    let mut library = library();
    let mut birth_summary = archetype(&library, BIRTH_SUMMARY).clone();
    let open_exclude: ArchetypeSlot = openehr_its::xml::runtime::from_xml(
        "<children xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"ARCHETYPE_SLOT\">\
         <rm_type_name>CLUSTER</rm_type_name><occurrences><lower_included>true</lower_included>\
         <lower_unbounded>false</lower_unbounded><upper_unbounded>true</upper_unbounded>\
         <lower>0</lower></occurrences><node_id>at0005</node_id>\
         <includes><expression xsi:type=\"EXPR_BINARY_OPERATOR\"><type>Boolean</type>\
         <operator>2007</operator><precedence_overridden>false</precedence_overridden>\
         <left_operand xsi:type=\"EXPR_LEAF\"><type>String</type>\
         <item xsi:type=\"xsd:string\">archetype_id/value</item>\
         <reference_type>attribute</reference_type></left_operand>\
         <right_operand xsi:type=\"EXPR_LEAF\"><type>C_STRING</type>\
         <item xsi:type=\"C_STRING\"><pattern>openEHR-EHR-CLUSTER\\.address\\.v1</pattern></item>\
         <reference_type>constraint</reference_type></right_operand></expression></includes>\
         <excludes><expression xsi:type=\"EXPR_BINARY_OPERATOR\"><type>Boolean</type>\
         <operator>2007</operator><precedence_overridden>false</precedence_overridden>\
         <left_operand xsi:type=\"EXPR_LEAF\"><type>String</type>\
         <item xsi:type=\"xsd:string\">archetype_id/value</item>\
         <reference_type>attribute</reference_type></left_operand>\
         <right_operand xsi:type=\"EXPR_LEAF\"><type>C_STRING</type>\
         <item xsi:type=\"C_STRING\"><pattern>.*</pattern></item>\
         <reference_type>constraint</reference_type></right_operand></expression></excludes>\
         </children>",
    )
    .expect("a well-formed slot");
    replace_slot(
        &mut birth_summary.definition.attributes,
        "at0005",
        &open_exclude,
    );
    library.insert(birth_summary);

    assert!(openehr_its::opt14_writer::build(&birth_summary_with_fills(), &library).is_ok());
    let mut definition = birth_summary_with_fills();
    definition.root.fills[0].filler = archetype_use(DEVICE);
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::SlotRefusesFiller { filler, .. }) if filler == DEVICE
    ));
}

fn replace_slot(attributes: &mut [CAttribute], node_id: &str, slot: &ArchetypeSlot) {
    for attribute in attributes {
        let children = match attribute {
            CAttribute::CMultipleAttribute(a) => &mut a.children,
            CAttribute::CSingleAttribute(a) => &mut a.children,
        };
        for child in children.iter_mut() {
            match child {
                CObject::ArchetypeSlot(s) if s.node_id == node_id => {
                    *child = CObject::ArchetypeSlot(slot.clone());
                }
                CObject::CComplexObject(o) => {
                    replace_slot(&mut o.attributes, node_id, slot);
                }
                _ => {}
            }
        }
    }
}

#[test]
fn a_filler_of_another_type_is_refused() {
    let library = library();
    let mut definition = birth_summary_with_fills();
    definition.root.fills[0].filler = archetype_use("openEHR-EHR-EVALUATION.clinical_synopsis.v1");
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::FillerTypeMismatch { slot_type, filler_type, .. })
            if slot_type == "CLUSTER" && filler_type == "EVALUATION"
    ));
}

#[test]
fn a_fill_that_names_no_slot_is_refused() {
    let library = library();
    let mut definition = birth_summary_with_fills();
    definition.root.fills[0].path = "/data[at0001]".to_owned();
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::NotASlot { .. })
    ));
    definition.root.fills[0].path = "/data[at0001]/items[at9999]".to_owned();
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::PathNotFound { found: 0, .. })
    ));
    definition.root.fills[0].path = "data[at0001]".to_owned();
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::MalformedPath { .. })
    ));
}

#[test]
fn narrowings_apply_and_widenings_are_refused() {
    let library = library();
    let mut definition = template("Birth summary", archetype_use(BIRTH_SUMMARY));
    definition.root.narrowings.push(Narrowing::Occurrences {
        path: PLACE_OF_BIRTH_SLOT.to_owned(),
        occurrences: closed(0, 1),
    });
    let built =
        openehr_its::opt14_writer::build(&definition, &library).expect("a narrowing builds");
    let slot = items_children(&built)
        .iter()
        .find(|c| head(c).2 == "at0005")
        .expect("the slot");
    assert_eq!(head(slot).1, &closed(0, 1));

    definition.root.narrowings = vec![Narrowing::Occurrences {
        path: "/data[at0001]".to_owned(),
        occurrences: closed(0, 1),
    }];
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::Widening {
            what: "occurrences",
            ..
        })
    ));
    definition.root.narrowings = vec![Narrowing::Exclude {
        path: "/data[at0001]".to_owned(),
    }];
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::Widening { .. })
    ));
}

#[test]
fn an_existence_of_zero_removes_the_attribute() {
    let library = library();
    let id = "openEHR-EHR-EVALUATION.clinical_synopsis.v1";
    let mut definition = template("Synopsis", archetype_use(id));
    definition.root.narrowings.push(Narrowing::Existence {
        path: "/protocol".to_owned(),
        existence: closed(0, 0),
    });
    let built = openehr_its::opt14_writer::build(&definition, &library).expect("builds");
    assert!(
        !built
            .definition
            .attributes
            .iter()
            .any(|a| attribute_parts(a).0 == "protocol"),
        "the protocol attribute is removed"
    );
    definition.root.narrowings = vec![Narrowing::Existence {
        path: "/data".to_owned(),
        existence: closed(0, 1),
    }];
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::Widening {
            what: "existence",
            ..
        })
    ));
}

#[test]
fn a_missing_archetype_or_language_is_refused() {
    let library = library();
    let definition = template(
        "Missing",
        archetype_use("openEHR-EHR-EVALUATION.no_such.v1"),
    );
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::ArchetypeNotFound { archetype_id }) if archetype_id.ends_with("no_such.v1")
    ));
    let mut definition = birth_summary_with_fills();
    definition.language.code_string = "xx".to_owned();
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::MissingLanguage { language, .. }) if language == "xx"
    ));
}

#[test]
fn a_recursive_internal_reference_is_refused() {
    let mut library = library();
    let id = "openEHR-EHR-EVALUATION.clinical_synopsis.v1";
    let mut synopsis = archetype(&library, id).clone();
    let data = synopsis
        .definition
        .attributes
        .iter_mut()
        .find(|a| attribute_parts(a).0 == "data")
        .expect("data");
    let CAttribute::CSingleAttribute(data) = data else {
        panic!("data is single-valued");
    };
    let CObject::CComplexObject(tree) = &mut data.children[0] else {
        panic!("the ITEM_TREE");
    };
    let CAttribute::CMultipleAttribute(items) = &mut tree.attributes[0] else {
        panic!("items is a container");
    };
    items
        .children
        .push(CObject::ArchetypeInternalRef(ArchetypeInternalRef {
            rm_type_name: "ITEM_TREE".to_owned(),
            occurrences: closed(0, 1),
            node_id: String::new(),
            target_path: "/data[at0001]".to_owned(),
        }));
    library.insert(synopsis);
    let definition = template("Synopsis", archetype_use(id));
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::RecursiveInternalRef { .. })
    ));
}

const REPORT_RESULT: &str = "openEHR-EHR-COMPOSITION.report-result.v1";
const IMAGING_EXAM_RESULT: &str = "openEHR-EHR-OBSERVATION.imaging_exam_result.v1";

/// The shape of an imaging report: a result report composition, which leaves
/// `content` open, holding an imaging examination result with its series.
fn imaging_report() -> TemplateDefinition {
    let mut result = archetype_use(IMAGING_EXAM_RESULT);
    result.fills.push(SlotFill {
        path: "/protocol[at0025]/items[at0065]".to_owned(),
        occurrences: None,
        filler: archetype_use("openEHR-EHR-CLUSTER.imaging_series.v0"),
    });
    let mut root = archetype_use(REPORT_RESULT);
    root.fills.push(SlotFill {
        path: "/content".to_owned(),
        occurrences: Some(Intervalofinteger {
            lower_included: Some(true),
            upper_included: None,
            lower_unbounded: false,
            upper_unbounded: true,
            lower: Some(1),
            upper: None,
        }),
        filler: result,
    });
    template("Imaging report", root)
}

#[test]
fn an_archetype_takes_an_open_attribute_as_the_reference_model_declares_it() {
    let library = library();
    let built = openehr_its::opt14_writer::build(&imaging_report(), &library).expect("builds");
    let content = built
        .definition
        .attributes
        .iter()
        .find(|a| attribute_parts(a).0 == "content")
        .expect("content is constrained");
    let CAttribute::CMultipleAttribute(content) = content else {
        panic!("COMPOSITION.content is a List in the RM");
    };
    assert_eq!(content.existence, closed(0, 1));
    assert!(content.cardinality.is_ordered);
    assert!(!content.cardinality.is_unique);
    let [CObject::CArchetypeRoot(result)] = content.children.as_slice() else {
        panic!("one archetype root in content");
    };
    assert_eq!(result.archetype_id.value, IMAGING_EXAM_RESULT);
    let components: Vec<&str> = built
        .component_ontologies
        .iter()
        .map(|o| o.archetype_id.as_str())
        .collect();
    assert_eq!(
        components,
        [
            REPORT_RESULT,
            IMAGING_EXAM_RESULT,
            "openEHR-EHR-CLUSTER.imaging_series.v0"
        ]
    );

    let xml = openehr_its::opt14::to_xml(&built).expect("serializes");
    assert_eq!(
        openehr_its::opt14::from_xml(&xml).expect("reads back"),
        built
    );
    let dir = scratch("imaging");
    let document = dir.join("imaging-report.opt");
    std::fs::write(&document, xml).expect("write OPT");
    let failures = xsd_failures(&template_xsd(), std::slice::from_ref(&document));
    std::fs::remove_dir_all(&dir).expect("remove scratch dir");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn an_open_attribute_takes_only_an_archetype_of_its_type() {
    let library = library();
    let mut definition = imaging_report();
    definition.root.fills[0].filler = archetype_use(ADDRESS);
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::FillerTypeMismatch { slot_type, .. }) if slot_type == "CONTENT_ITEM"
    ));
    definition.root.fills[0].path = "/no_such_attribute".to_owned();
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::AttributeNotInModel { .. })
    ));
    let mut definition = imaging_report();
    definition.root.fills[0].path = "/context".to_owned();
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::AttributeConstrained { .. })
    ));
}

#[test]
fn a_template_constrains_an_open_attribute() {
    let library = library();
    let name: CAttribute = openehr_its::xml::runtime::from_xml(
        "<attributes xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"C_SINGLE_ATTRIBUTE\">\
         <rm_attribute_name>name</rm_attribute_name><existence><lower_included>true</lower_included>\
         <upper_included>true</upper_included><lower_unbounded>false</lower_unbounded>\
         <upper_unbounded>false</upper_unbounded><lower>1</lower><upper>1</upper></existence>\
         </attributes>",
    )
    .expect("a well-formed attribute");
    let mut definition = template("Birth summary", archetype_use(BIRTH_SUMMARY));
    definition.root.narrowings.push(Narrowing::Constrain {
        path: "/".to_owned(),
        attribute: name.clone(),
    });
    definition.root.narrowings.push(Narrowing::Constrain {
        path: "/data[at0001]/items[at0002]".to_owned(),
        attribute: name.clone(),
    });
    let built = openehr_its::opt14_writer::build(&definition, &library).expect("builds");
    assert!(
        built
            .definition
            .attributes
            .iter()
            .any(|a| attribute_parts(a).0 == "name")
    );
    definition.root.narrowings.push(Narrowing::Constrain {
        path: "/".to_owned(),
        attribute: name.clone(),
    });
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::AttributeConstrained { .. })
    ));
    let mut bogus = name;
    if let CAttribute::CSingleAttribute(a) = &mut bogus {
        "no_such_attribute".clone_into(&mut a.rm_attribute_name);
    }
    definition.root.narrowings = vec![Narrowing::Constrain {
        path: "/".to_owned(),
        attribute: bogus,
    }];
    assert!(matches!(
        openehr_its::opt14_writer::build(&definition, &library),
        Err(WriteError::AttributeNotInModel { .. })
    ));
}

// ---------------------------------------------------------------------------
// Clones, redefinitions and mixed open attributes
// ---------------------------------------------------------------------------

const STREET: &str = "/items[at0001]";

fn interval_xml(tag: &str, lower: i32, upper: Option<i32>) -> String {
    match upper {
        Some(upper) => format!(
            "<{tag}><lower_included>true</lower_included><upper_included>true</upper_included>\
             <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
             <lower>{lower}</lower><upper>{upper}</upper></{tag}>"
        ),
        None => format!(
            "<{tag}><lower_included>true</lower_included><lower_unbounded>false</lower_unbounded>\
             <upper_unbounded>true</upper_unbounded><lower>{lower}</lower></{tag}>"
        ),
    }
}

/// A complex object constraint on `rm_type` with no attributes.
fn complex(rm_type: &str, node_id: &str) -> CObject {
    let occurrences = interval_xml("occurrences", 1, Some(1));
    openehr_its::xml::runtime::from_xml(&format!(
        "<children xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"C_COMPLEX_OBJECT\">\
         <rm_type_name>{rm_type}</rm_type_name>{occurrences}<node_id>{node_id}</node_id>\
         </children>"
    ))
    .expect("a well-formed object")
}

/// The `name` constraint a template gives a node: a `DV_TEXT` whose value is
/// `text`.
fn name_constraint(text: &str) -> CAttribute {
    let existence = interval_xml("existence", 1, Some(1));
    let occurrences = interval_xml("occurrences", 1, Some(1));
    openehr_its::xml::runtime::from_xml(&format!(
        "<attributes xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"C_SINGLE_ATTRIBUTE\">\
         <rm_attribute_name>name</rm_attribute_name>{existence}\
         <children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>DV_TEXT</rm_type_name>{occurrences}\
         <node_id/><attributes xsi:type=\"C_SINGLE_ATTRIBUTE\"><rm_attribute_name>value</rm_attribute_name>\
         {existence}<children xsi:type=\"C_PRIMITIVE_OBJECT\"><rm_type_name>STRING</rm_type_name>\
         {occurrences}<node_id/><item xsi:type=\"C_STRING\"><list>{text}</list></item></children>\
         </attributes></children></attributes>"
    ))
    .expect("a well-formed attribute")
}

/// Builds `definition`, checks its OPT against `Template.xsd`, reads it back
/// to the same model and writes the same bytes twice.
fn written(definition: &TemplateDefinition, library: &ArchetypeLibrary) -> OperationalTemplate {
    let built = openehr_its::opt14_writer::build(definition, library).expect("builds");
    let xml = openehr_its::opt14_writer::write(definition, library).expect("writes");
    assert_eq!(
        openehr_its::opt14_writer::write(definition, library).expect("writes"),
        xml,
        "the same definition writes the same bytes"
    );
    assert_eq!(
        openehr_its::opt14::from_xml(&xml).expect("reads back"),
        built,
        "the OPT reads back to the model the writer built"
    );
    let dir = scratch(&definition.template_id.value.replace(' ', "-"));
    let document = dir.join("template.opt");
    std::fs::write(&document, xml).expect("write OPT");
    let failures = xsd_failures(&template_xsd(), std::slice::from_ref(&document));
    std::fs::remove_dir_all(&dir).expect("remove scratch dir");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    built
}

fn root_items(template: &OperationalTemplate) -> &[CObject] {
    let items = template
        .definition
        .attributes
        .iter()
        .find(|a| attribute_parts(a).0 == "items")
        .expect("items");
    attribute_parts(items).2
}

fn name_of(object: &CObject) -> Option<&CAttribute> {
    own_attributes(object)
        .iter()
        .find(|a| attribute_parts(a).0 == "name")
}

fn value_types<'a>(items: &'a [CObject], node_id: &str) -> Vec<&'a str> {
    items
        .iter()
        .filter(|c| head(c).2 == node_id)
        .flat_map(|c| {
            own_attributes(c)
                .iter()
                .filter(|a| attribute_parts(a).0 == "value")
                .flat_map(|a| attribute_parts(a).2.iter().map(|v| head(v).0))
        })
        .collect()
}

#[test]
fn a_clone_repeats_a_node_under_its_node_id_after_it() {
    let library = library();
    let mut root = archetype_use(ADDRESS);
    root.narrowings.push(Narrowing::Constrain {
        path: STREET.to_owned(),
        attribute: name_constraint("Street"),
    });
    let mut second = NodeClone::new(STREET, name_constraint("Second street"));
    second.occurrences = Some(closed(0, 1));
    second.narrowings.push(Narrowing::Existence {
        path: "/value".to_owned(),
        existence: closed(1, 1),
    });
    root.narrowings.push(Narrowing::Clone(second));
    root.narrowings.push(Narrowing::Clone(NodeClone::new(
        STREET,
        name_constraint("Third street"),
    )));
    let built = written(&template("Address clones", root), &library);
    let items = root_items(&built);
    let at = items
        .iter()
        .position(|c| head(c).2 == "at0001")
        .expect("the node stays");
    let streets: Vec<&CObject> = items.iter().skip(at).take(3).collect();
    assert!(streets.iter().all(|c| head(c).2 == "at0001"));
    assert_eq!(items.iter().filter(|c| head(c).2 == "at0001").count(), 3);
    let names: Vec<Option<&CAttribute>> = streets.iter().map(|c| name_of(c)).collect();
    assert_eq!(
        names,
        [
            Some(&name_constraint("Street")),
            Some(&name_constraint("Second street")),
            Some(&name_constraint("Third street"))
        ]
    );
    assert_eq!(head(streets[1]).1, &closed(0, 1));
    let unbounded = head(streets[2]).1;
    assert!(
        unbounded.upper_unbounded,
        "a clone keeps the node's occurrences"
    );
    let value_existence = |c: &CObject| {
        own_attributes(c)
            .iter()
            .find(|a| attribute_parts(a).0 == "value")
            .map(|a| interval_text(attribute_parts(a).1))
    };
    assert_eq!(value_existence(streets[1]).as_deref(), Some("1..1"));
    assert_eq!(value_existence(streets[0]).as_deref(), Some("0..1"));
}

#[test]
fn a_clone_takes_the_place_of_an_excluded_node() {
    let library = library();
    let mut root = archetype_use(ADDRESS);
    root.narrowings.push(Narrowing::Clone(NodeClone::new(
        STREET,
        name_constraint("Street"),
    )));
    root.narrowings.push(Narrowing::Clone(NodeClone::new(
        STREET,
        name_constraint("Second street"),
    )));
    root.narrowings.push(Narrowing::Exclude {
        path: STREET.to_owned(),
    });
    let built = written(&template("Address renamed", root), &library);
    let streets: Vec<&CObject> = root_items(&built)
        .iter()
        .filter(|c| head(c).2 == "at0001")
        .collect();
    assert_eq!(streets.len(), 2);
    assert!(streets.iter().all(|c| name_of(c).is_some()));
}

#[test]
fn a_clone_the_archetype_does_not_admit_is_refused() {
    let library = library();
    let refusal = |library: &ArchetypeLibrary, clones: Vec<NodeClone>| {
        let mut root = archetype_use(ADDRESS);
        root.narrowings
            .extend(clones.into_iter().map(Narrowing::Clone));
        openehr_its::opt14_writer::build(&template("Address", root), library)
    };
    // `/items[at0004]` occurs at most once.
    assert!(matches!(
        refusal(
            &library,
            vec![NodeClone::new("/items[at0004]", name_constraint("x"))]
        ),
        Err(WriteError::CloneNotAdmitted { .. })
    ));
    assert!(matches!(
        refusal(
            &library,
            vec![
                NodeClone::new(STREET, name_constraint("Street")),
                NodeClone::new(STREET, name_constraint("Street")),
            ]
        ),
        Err(WriteError::CloneNameNotDistinct { .. })
    ));
    let mut not_a_name = name_constraint("Street");
    if let CAttribute::CSingleAttribute(a) = &mut not_a_name {
        "value".clone_into(&mut a.rm_attribute_name);
    }
    assert!(matches!(
        refusal(&library, vec![NodeClone::new(STREET, not_a_name)]),
        Err(WriteError::CloneNotAdmitted { .. })
    ));
    assert!(matches!(
        refusal(
            &library,
            vec![NodeClone::new("/items", name_constraint("x"))]
        ),
        Err(WriteError::MalformedPath { .. })
    ));

    // The items cardinality narrowed to {0..2}: no copy may occur three
    // times, and three mandatory members do not fit.
    let mut narrow = library.clone();
    let mut address = archetype(&library, ADDRESS).clone();
    for attribute in &mut address.definition.attributes {
        if let CAttribute::CMultipleAttribute(items) = attribute {
            items.cardinality.interval = closed(0, 2);
        }
    }
    narrow.insert(address);
    let mut too_many = NodeClone::new(STREET, name_constraint("Street"));
    too_many.occurrences = Some(closed(0, 3));
    assert!(matches!(
        refusal(&narrow, vec![too_many]),
        Err(WriteError::CloneNotAdmitted { .. })
    ));
    let mandatory = |text| {
        let mut clone = NodeClone::new(STREET, name_constraint(text));
        clone.occurrences = Some(closed(1, 1));
        clone
    };
    assert!(matches!(
        refusal(
            &narrow,
            vec![mandatory("a"), mandatory("b"), mandatory("c")]
        ),
        Err(WriteError::CloneNotAdmitted { .. })
    ));
    assert!(refusal(&narrow, vec![mandatory("a")]).is_ok());
}

#[test]
fn a_redefinition_narrows_a_type_and_drops_alternatives() {
    let library = library();
    let mut root = archetype_use(ADDRESS);
    root.narrowings.push(Narrowing::Redefine {
        path: format!("{STREET}/value"),
        objects: vec![complex("DV_CODED_TEXT", "")],
    });
    root.narrowings.push(Narrowing::Redefine {
        path: "/items[at0010]/value".to_owned(),
        objects: vec![complex("DV_TEXT", "")],
    });
    let built = written(&template("Address redefined", root), &library);
    let items = root_items(&built);
    assert_eq!(value_types(items, "at0001"), ["DV_CODED_TEXT"]);
    assert_eq!(value_types(items, "at0010"), ["DV_TEXT"]);
    assert_eq!(value_types(items, "at0014"), ["DV_CODED_TEXT", "DV_TEXT"]);
}

#[test]
fn a_redefinition_that_does_not_narrow_is_refused() {
    let library = library();
    let refusal = |path: &str, object: CObject| {
        let mut root = archetype_use(ADDRESS);
        root.narrowings.push(Narrowing::Redefine {
            path: path.to_owned(),
            objects: vec![object],
        });
        openehr_its::opt14_writer::build(&template("Address", root), &library)
    };
    let value = format!("{STREET}/value");
    assert!(matches!(
        refusal(&value, complex("DV_COUNT", "")),
        Err(WriteError::TypeNotConforming { given, archetype, .. })
            if given == "DV_COUNT" && archetype == "DV_TEXT"
    ));
    assert!(matches!(
        refusal("/items", complex("ELEMENT", "at9999")),
        Err(WriteError::RedefinitionUnmatched { .. })
    ));
    assert!(matches!(
        refusal("/items", complex("CLUSTER", "at0001")),
        Err(WriteError::TypeNotConforming { .. })
    ));
    let mut widened = complex("DV_TEXT", "");
    if let CObject::CComplexObject(o) = &mut widened {
        o.occurrences = closed(0, 2);
    }
    assert!(matches!(
        refusal(&value, widened),
        Err(WriteError::Widening { .. })
    ));
}

#[test]
fn an_open_attribute_holds_archetypes_beside_a_constraint() {
    let library = library();
    let mut definition = imaging_report();
    let existence = interval_xml("existence", 0, Some(1));
    let cardinality = interval_xml("interval", 0, None);
    let occurrences = interval_xml("occurrences", 0, None);
    let content: CAttribute = openehr_its::xml::runtime::from_xml(&format!(
        "<attributes xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"C_MULTIPLE_ATTRIBUTE\">\
         <rm_attribute_name>content</rm_attribute_name>{existence}\
         <children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>SECTION</rm_type_name>{occurrences}\
         <node_id/></children><cardinality><is_ordered>true</is_ordered>\
         <is_unique>false</is_unique>{cardinality}</cardinality></attributes>"
    ))
    .expect("a well-formed attribute");
    definition.root.narrowings.push(Narrowing::Constrain {
        path: "/".to_owned(),
        attribute: content,
    });
    let built = written(&definition, &library);
    let content = built
        .definition
        .attributes
        .iter()
        .find(|a| attribute_parts(a).0 == "content")
        .expect("content");
    let kinds: Vec<&str> = attribute_parts(content).2.iter().map(kind).collect();
    assert_eq!(kinds, ["C_COMPLEX_OBJECT", "C_ARCHETYPE_ROOT"]);
}

#[test]
fn a_clone_of_a_node_that_cannot_repeat_is_refused() {
    let library = library();
    let build = |id: &str, clone: NodeClone| {
        let mut root = archetype_use(id);
        root.narrowings.push(Narrowing::Clone(clone));
        openehr_its::opt14_writer::build(&template("Clone", root), &library)
    };
    assert!(
        matches!(
            build(BIRTH_SUMMARY, NodeClone::new("/data[at0001]", name_constraint("x"))),
            Err(WriteError::CloneNotAdmitted { reason, .. }) if reason.contains("single-valued")
        ),
        "a child of a single-valued attribute has no second copy"
    );
    assert!(
        matches!(
            build(
                ADDRESS,
                NodeClone::new("/items[at0020]", name_constraint("x"))
            ),
            Err(WriteError::NotAComplexObject { .. })
        ),
        "a slot is no complex object to name"
    );
    let mut renamed = NodeClone::new(STREET, name_constraint("x"));
    renamed.narrowings.push(Narrowing::Constrain {
        path: "/".to_owned(),
        attribute: name_constraint("y"),
    });
    assert!(
        matches!(
            build(ADDRESS, renamed),
            Err(WriteError::AttributeConstrained { .. })
        ),
        "a copy carries its clone name already"
    );
}

fn single(name: &str, existence: Intervalofinteger) -> CAttribute {
    CAttribute::CSingleAttribute(CSingleAttribute {
        rm_attribute_name: name.to_owned(),
        existence,
        children: Vec::new(),
    })
}

fn element_with(attribute: CAttribute) -> CObject {
    let mut element = complex("ELEMENT", "at0001");
    if let CObject::CComplexObject(o) = &mut element {
        o.occurrences = closed(0, 1);
        o.attributes.push(attribute);
    }
    element
}

#[test]
fn a_redefinition_overlays_the_attributes_it_names() {
    let library = library();
    let redefine = |object: CObject| {
        let mut root = archetype_use(ADDRESS);
        root.narrowings.push(Narrowing::Redefine {
            path: "/items".to_owned(),
            objects: vec![object],
        });
        openehr_its::opt14_writer::build(&template("Address", root), &library)
    };
    let built = redefine(element_with(single("value", closed(0, 0)))).expect("builds");
    let street = root_items(&built)
        .iter()
        .find(|c| head(c).2 == "at0001")
        .expect("the street");
    assert!(
        own_attributes(street)
            .iter()
            .all(|a| attribute_parts(a).0 != "value"),
        "an attribute at existence {{0}} is removed"
    );
    assert!(
        root_items(&built).iter().any(|c| head(c).2 == "at0002"),
        "a named node no object redefines stays"
    );
    assert!(
        matches!(
            redefine(element_with(single("value", closed(0, 2)))),
            Err(WriteError::Widening {
                what: "existence",
                ..
            })
        ),
        "a wider existence is refused"
    );
    assert!(
        matches!(
            redefine(element_with(single("no_such_attribute", closed(0, 1)))),
            Err(WriteError::AttributeNotInModel { .. })
        ),
        "an attribute the type lacks is refused"
    );
    let multiple = CAttribute::CMultipleAttribute(CMultipleAttribute {
        rm_attribute_name: "value".to_owned(),
        existence: closed(0, 1),
        children: Vec::new(),
        cardinality: Cardinality {
            is_ordered: false,
            is_unique: false,
            interval: closed(0, 1),
        },
    });
    assert!(
        matches!(
            redefine(element_with(multiple)),
            Err(WriteError::RedefinitionUnmatched { .. })
        ),
        "a single-valued attribute stays single-valued"
    );
    let mut excluded = element_with(single("value", closed(0, 1)));
    if let CObject::CComplexObject(o) = &mut excluded {
        o.occurrences = closed(0, 0);
    }
    let built = redefine(excluded).expect("builds");
    assert!(
        root_items(&built).iter().all(|c| head(c).2 != "at0001"),
        "an object at occurrences {{0}} removes the node"
    );
}
