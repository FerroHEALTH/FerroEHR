// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! XSD reader for the XML codegen: per `xs:complexType`, attributes versus
//! child elements, child order, `base` and `abstract`, plus the
//! `xs:enumeration` facets of named simple types. `xs:group` references expand
//! in place. Each caller curates a conflict-free file set (the constants
//! below).

use std::collections::BTreeMap;
use std::path::Path;

/// A parsed XSD type model: openEHR complexType name → [`XsdType`].
pub(crate) struct XsdModel {
    /// Target XML namespace (`http://schemas.openehr.org/v1` or `…/v2`).
    pub namespace: String,
    pub types: BTreeMap<String, XsdType>,
    /// Named `xs:simpleType` definitions, keyed by name.
    pub simple_types: BTreeMap<String, XsdSimpleType>,
}

/// One named `xs:simpleType`: its restriction base and its `xs:enumeration`
/// facets.
pub(crate) struct XsdSimpleType {
    pub name: String,
    /// The `xs:restriction` `base` (`xs:integer`, `xs:string`, …), empty when
    /// the type is an `xs:union`/`xs:list` rather than a restriction.
    pub base: String,
    /// The declared `xs:enumeration` facets, in declaration order.
    pub enumerations: Vec<XsdEnumFacet>,
}

/// One `<xs:enumeration>` facet of a named simple type.
pub(crate) struct XsdEnumFacet {
    /// The facet's `value` — the lexical form a document carries.
    pub value: String,
    /// The facet's `id` annotation (`<xs:enumeration value="2001" id="equal"/>`).
    pub id: Option<String>,
}

/// One `xs:complexType` (its *local* attributes/elements — inheritance is via
/// [`XsdType::base`], resolved by [`XsdModel::flattened`]).
pub(crate) struct XsdType {
    pub name: String,
    pub is_abstract: bool,
    /// The `xs:extension`/`xs:restriction` base type, if any.
    pub base: Option<String>,
    /// Local attributes, declaration order.
    pub attributes: Vec<XsdAttr>,
    /// Local child elements, sequence order.
    pub elements: Vec<XsdElem>,
}

#[derive(Clone)]
pub(crate) struct XsdAttr {
    pub name: String,
    /// The declared `type` of the XSD attribute.
    pub type_name: String,
    pub required: bool,
}

#[derive(Clone)]
pub(crate) struct XsdElem {
    pub name: String,
    pub type_name: String,
    pub optional: bool,
    pub multiple: bool,
}

/// Collects one document's named `group` / `attributeGroup` definitions; first
/// declaration wins.
fn collect_groups(doc: &roxmltree::Document<'_>, groups: &mut XsdGroups) {
    for node in doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
    {
        let Some(name) = node.attribute("name") else {
            continue;
        };
        match local(&node) {
            "group" => {
                let mut items = Vec::new();
                collect_content_particles(node, &mut items, &mut Vec::new());
                groups.elements.entry(name.to_owned()).or_insert(items);
            }
            "attributeGroup" => {
                let mut items = Vec::new();
                collect_content_particles(node, &mut Vec::new(), &mut items);
                groups.attributes.entry(name.to_owned()).or_insert(items);
            }
            _ => {}
        }
    }
}

/// Collects one document's `complexType`s (with every group reference expanded
/// in place) and its named `simpleType`s.
///
/// # Errors
/// A complexType the parser cannot resolve.
fn collect_types(
    doc: &roxmltree::Document<'_>,
    groups: &XsdGroups,
    types: &mut BTreeMap<String, XsdType>,
    simple_types: &mut BTreeMap<String, XsdSimpleType>,
) -> Result<(), String> {
    for node in doc
        .root_element()
        .children()
        .filter(roxmltree::Node::is_element)
    {
        match local(&node) {
            "complexType" => {
                if let Some(t) = parse_complex_type(node, groups)? {
                    types.entry(t.name.clone()).or_insert(t);
                }
            }
            "simpleType" => {
                if let Some(t) = parse_simple_type(node) {
                    simple_types.entry(t.name.clone()).or_insert(t);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

impl XsdModel {
    /// Parses a curated set of XSD files into one model, first declaration wins.
    ///
    /// # Errors
    /// Returns an error if a file cannot be read or parsed as XML.
    pub(crate) fn parse_files(paths: &[std::path::PathBuf]) -> Result<Self, String> {
        // Groups first: one may be referenced from another file of the include chain.
        let mut groups = XsdGroups::default();
        for path in paths {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let doc = roxmltree::Document::parse(&text)
                .map_err(|e| format!("parse {}: {e}", path.display()))?;
            collect_groups(&doc, &mut groups);
        }

        // Pass 2: the complexTypes, with every group reference expanded in
        // place, plus the named simpleTypes.
        let mut types: BTreeMap<String, XsdType> = BTreeMap::new();
        let mut simple_types: BTreeMap<String, XsdSimpleType> = BTreeMap::new();
        let mut namespace = String::new();
        for path in paths {
            let text = std::fs::read_to_string(path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            let doc = roxmltree::Document::parse(&text)
                .map_err(|e| format!("parse {}: {e}", path.display()))?;
            let root = doc.root_element();
            if namespace.is_empty()
                && let Some(ns) = root.attribute("targetNamespace")
            {
                namespace = ns.to_string();
            }
            collect_types(&doc, &groups, &mut types, &mut simple_types)
                .map_err(|e| format!("{}: {e}", path.display()))?;
        }
        Ok(Self {
            namespace,
            types,
            simple_types,
        })
    }

    /// The flattened attributes and elements of a concrete type, ancestors first
    /// within each.
    #[must_use]
    pub(crate) fn flattened(&self, name: &str) -> (Vec<XsdAttr>, Vec<XsdElem>) {
        // Collect the chain root→…→self, then concatenate in that order.
        let mut chain: Vec<&XsdType> = Vec::new();
        let mut cur = self.types.get(name);
        while let Some(t) = cur {
            chain.push(t);
            cur = t.base.as_deref().and_then(|b| self.types.get(b));
        }
        chain.reverse(); // ancestor-first
        let mut attrs = Vec::new();
        let mut elems = Vec::new();
        for t in chain {
            attrs.extend(t.attributes.iter().cloned());
            elems.extend(t.elements.iter().cloned());
        }
        (attrs, elems)
    }

    /// The concrete descendants of `name` (types whose `base` chain reaches
    /// `name`), i.e. the valid `xsi:type` values for a slot declared as `name`.
    #[must_use]
    pub(crate) fn descendants(&self, name: &str) -> Vec<String> {
        self.types
            .values()
            .filter(|t| !t.is_abstract && self.is_a(&t.name, name))
            .map(|t| t.name.clone())
            .collect()
    }

    /// Whether `sub` is `sup` or transitively extends it.
    #[must_use]
    pub(crate) fn is_a(&self, sub: &str, sup: &str) -> bool {
        let mut cur = Some(sub.to_string());
        while let Some(n) = cur {
            if n == sup {
                return true;
            }
            cur = self.types.get(&n).and_then(|t| t.base.clone());
        }
        false
    }
}

/// One item of an XSD content particle, in declaration order.
enum Particle {
    /// A named `xs:element` declaration.
    Element(XsdElem),
    /// `<xs:group ref="NAME"/>`, with whether the reference repeats.
    GroupRef(String, bool),
}

/// One item of an XSD attribute list, in declaration order.
enum AttrParticle {
    /// A named `xs:attribute` declaration.
    Attr(XsdAttr),
    /// `<xs:attributeGroup ref="NAME"/>`, expanded in place.
    GroupRef(String),
}

/// The named `xs:group` / `xs:attributeGroup` definitions of one curated file set.
#[derive(Default)]
struct XsdGroups {
    /// `xs:group` name → its content particle.
    elements: BTreeMap<String, Vec<Particle>>,
    /// `xs:attributeGroup` name → its attribute list.
    attributes: BTreeMap<String, Vec<AttrParticle>>,
}

impl XsdGroups {
    /// Flattens `items` into `out`, expanding every `xs:group` reference and
    /// reporting a group cycle.
    fn expand_elements(
        &self,
        items: &[Particle],
        repeat: bool,
        visiting: &mut Vec<String>,
        out: &mut Vec<XsdElem>,
    ) -> Result<(), String> {
        for item in items {
            match item {
                Particle::Element(e) => {
                    let mut e = e.clone();
                    e.multiple = e.multiple || repeat;
                    out.push(e);
                }
                Particle::GroupRef(name, group_repeat) => {
                    if visiting.iter().any(|n| n == name) {
                        return Err(format!(
                            "xs:group {name:?} is cyclic (expansion stack {visiting:?})"
                        ));
                    }
                    let body = self.elements.get(name).ok_or_else(|| {
                        format!("xs:group ref {name:?} has no definition in this schema closure")
                    })?;
                    visiting.push(name.clone());
                    self.expand_elements(body, repeat || *group_repeat, visiting, out)?;
                    visiting.pop();
                }
            }
        }
        Ok(())
    }

    /// Flatten `items` into `out`, expanding every `xs:attributeGroup` reference.
    fn expand_attributes(
        &self,
        items: &[AttrParticle],
        visiting: &mut Vec<String>,
        out: &mut Vec<XsdAttr>,
    ) -> Result<(), String> {
        for item in items {
            match item {
                AttrParticle::Attr(a) => out.push(a.clone()),
                AttrParticle::GroupRef(name) => {
                    if visiting.iter().any(|n| n == name) {
                        return Err(format!(
                            "xs:attributeGroup {name:?} is cyclic (expansion stack {visiting:?})"
                        ));
                    }
                    let body = self.attributes.get(name).ok_or_else(|| {
                        format!(
                            "xs:attributeGroup ref {name:?} has no definition in this schema closure"
                        )
                    })?;
                    visiting.push(name.clone());
                    self.expand_attributes(body, visiting, out)?;
                    visiting.pop();
                }
            }
        }
        Ok(())
    }
}

fn local<'input>(n: &roxmltree::Node<'_, 'input>) -> &'input str {
    n.tag_name().name()
}

fn parse_complex_type(
    node: roxmltree::Node,
    groups: &XsdGroups,
) -> Result<Option<XsdType>, String> {
    let Some(name) = node.attribute("name") else {
        return Ok(None);
    };
    let is_abstract = node.attribute("abstract") == Some("true");
    let mut base = None;
    let mut elems: Vec<Particle> = Vec::new();
    let mut attrs: Vec<AttrParticle> = Vec::new();
    // Content is either wrapped in complexContent/simpleContent > extension|restriction,
    // or a direct sequence/choice/all + attributes on the complexType itself.
    for child in node.children().filter(roxmltree::Node::is_element) {
        match local(&child) {
            "complexContent" | "simpleContent" => {
                for deriv in child.children().filter(roxmltree::Node::is_element) {
                    if matches!(local(&deriv), "extension" | "restriction") {
                        base = deriv.attribute("base").map(str::to_string);
                        collect_content_particles(deriv, &mut elems, &mut attrs);
                    }
                }
            }
            _ => collect_particle_item(child, &mut elems, &mut attrs),
        }
    }
    let mut ty = XsdType {
        name: name.to_string(),
        is_abstract,
        base,
        attributes: Vec::new(),
        elements: Vec::new(),
    };
    groups
        .expand_elements(&elems, false, &mut Vec::new(), &mut ty.elements)
        .map_err(|e| format!("complexType {name:?}: {e}"))?;
    groups
        .expand_attributes(&attrs, &mut Vec::new(), &mut ty.attributes)
        .map_err(|e| format!("complexType {name:?}: {e}"))?;
    Ok(Some(ty))
}

/// Parses one named `xs:simpleType`; a union or list yields an empty base and
/// no facets.
fn parse_simple_type(node: roxmltree::Node) -> Option<XsdSimpleType> {
    let name = node.attribute("name")?;
    let restriction = node
        .children()
        .filter(roxmltree::Node::is_element)
        .find(|c| local(c) == "restriction");
    let Some(restriction) = restriction else {
        return Some(XsdSimpleType {
            name: name.to_owned(),
            base: String::new(),
            enumerations: Vec::new(),
        });
    };
    let enumerations = restriction
        .children()
        .filter(roxmltree::Node::is_element)
        .filter(|f| local(f) == "enumeration")
        .filter_map(|f| {
            f.attribute("value").map(|value| XsdEnumFacet {
                value: value.to_owned(),
                id: f.attribute("id").map(str::to_owned),
            })
        })
        .collect();
    Some(XsdSimpleType {
        name: name.to_owned(),
        base: restriction.attribute("base").unwrap_or("").to_owned(),
        enumerations,
    })
}

/// Collect the element/attribute content directly under a container
/// (`xs:extension`/`xs:restriction`, a complexType, or an `xs:group` definition).
fn collect_content_particles(
    container: roxmltree::Node,
    elems: &mut Vec<Particle>,
    attrs: &mut Vec<AttrParticle>,
) {
    for child in container.children().filter(roxmltree::Node::is_element) {
        collect_particle_item(child, elems, attrs);
    }
}

/// Dispatch one child of a content container.
fn collect_particle_item(
    child: roxmltree::Node,
    elems: &mut Vec<Particle>,
    attrs: &mut Vec<AttrParticle>,
) {
    match local(&child) {
        "sequence" | "choice" | "all" => collect_particle(child, elems),
        "group" => push_group_ref(child, elems),
        "attribute" => push_attr(child, attrs),
        "attributeGroup" => {
            if let Some(name) = child.attribute("ref") {
                attrs.push(AttrParticle::GroupRef(name.to_string()));
            }
        }
        _ => {}
    }
}

/// Collects a particle's `xs:element`s and `xs:group` references in order.
fn collect_particle(particle: roxmltree::Node, out: &mut Vec<Particle>) {
    for child in particle.children().filter(roxmltree::Node::is_element) {
        match local(&child) {
            "element" => {
                let Some(name) = child.attribute("name") else {
                    continue; // ref-based element: not used by the RM instance schemas
                };
                let type_name = child.attribute("type").unwrap_or("").to_string();
                let min = child.attribute("minOccurs").unwrap_or("1");
                out.push(Particle::Element(XsdElem {
                    name: name.to_string(),
                    type_name,
                    optional: min == "0",
                    multiple: is_repeatable(child),
                }));
            }
            "group" => push_group_ref(child, out),
            "sequence" | "choice" | "all" => collect_particle(child, out),
            _ => {}
        }
    }
}

/// Record a `<xs:group ref="…"/>` reference for later expansion.
fn push_group_ref(node: roxmltree::Node, out: &mut Vec<Particle>) {
    if let Some(name) = node.attribute("ref") {
        out.push(Particle::GroupRef(name.to_string(), is_repeatable(node)));
    }
}

/// Whether a particle declaration admits more than one occurrence.
fn is_repeatable(node: roxmltree::Node) -> bool {
    let max = node.attribute("maxOccurs").unwrap_or("1");
    max == "unbounded" || max.parse::<u32>().unwrap_or(1) > 1
}

fn push_attr(node: roxmltree::Node, out: &mut Vec<AttrParticle>) {
    let Some(name) = node.attribute("name") else {
        return;
    };
    out.push(AttrParticle::Attr(XsdAttr {
        name: name.to_string(),
        type_name: node.attribute("type").unwrap_or("").to_string(),
        required: node.attribute("use") == Some("required"),
    }));
}

/// The RM *instance* XSD file basenames per lineage (order = merge order).
/// Excludes the OPT/AOM constraint schemas that redefine RM type names.
pub(crate) const RM_FILES_V1: &[&str] = &[
    "BaseTypes.xsd",
    "Structure.xsd",
    "Content.xsd",
    "Composition.xsd",
    "Version.xsd",
    "Extract.xsd",
    "Resource.xsd",
];

/// Resolve the v1 RM-instance file paths under the `ALL/` bundle dir.
#[must_use]
pub(crate) fn v1_files(all_dir: &Path) -> Vec<std::path::PathBuf> {
    RM_FILES_V1.iter().map(|f| all_dir.join(f)).collect()
}

/// The v1 `ALL/` files of the served RM closure, merged first.
///
/// `Extract.xsd` is excluded: its RM 1.0.2 `EXTRACT_ITEM` does not extend
/// `LOCATABLE`, so the extract family comes from [`RM_FILES_V2_SUPPLEMENT`].
pub(crate) const RM_FILES_V1_SERVED: &[&str] = &[
    "BaseTypes.xsd",
    "Structure.xsd",
    "Content.xsd",
    "Composition.xsd",
    "Version.xsd",
    "Resource.xsd",
];

/// The v2 (RM 1.1.0) schemas for the types the v1 `ALL/` bundle lacks (EHR,
/// demographic) or carries stale (extract), merged after the v1 core.
pub(crate) const RM_FILES_V2_SUPPLEMENT: &[&str] = &[
    "RM/Release-1.1.0/Ehr.xsd",
    "RM/Release-1.1.0/Demographic.xsd",
    "RM/Release-1.1.0/EhrExtract.xsd",
];

/// The merged emit-xml XSD input: the v1 core, then the v2 supplement.
#[must_use]
pub(crate) fn xml_emit_files(v1_all_dir: &Path, v2_root: &Path) -> Vec<std::path::PathBuf> {
    RM_FILES_V1_SERVED
        .iter()
        .map(|f| v1_all_dir.join(f))
        .chain(RM_FILES_V2_SUPPLEMENT.iter().map(|f| v2_root.join(f)))
        .collect()
}

/// The AM/OPT 1.4 closure (`Template.xsd` and its includes), `Template.xsd`
/// first.
pub(crate) const AM_FILES_V1: &[&str] = &[
    "Template.xsd",
    "OpenehrProfile.xsd",
    "Archetype.xsd",
    "Resource.xsd",
    "BaseTypes.xsd",
];

/// Resolve the v1 AM/OPT constraint-schema file paths under the `ALL/` bundle dir.
#[must_use]
pub(crate) fn am_files_v1(all_dir: &Path) -> Vec<std::path::PathBuf> {
    AM_FILES_V1.iter().map(|f| all_dir.join(f)).collect()
}

/// The AOM2 persistent-form closure (`P_Archetype.xsd` and its includes), the
/// form the bundle's `examples/*.xml` use.
///
/// NOTE: `Archetype.xsd` declares `archetype` with a different root type, so it
/// is a separate closure ([`AOM2_MODEL_FILES`]).
pub(crate) const AOM2_FILES: &[&str] = &[
    "P_Archetype.xsd",
    "ArchetypeCommon.xsd",
    "Rules.xsd",
    "Resource.xsd",
    "BaseTypes.xsd",
];

/// Resolve the AOM2 schema file paths under the bundle's `AOM2/` dir.
#[must_use]
pub(crate) fn aom2_files(aom2_dir: &Path) -> Vec<std::path::PathBuf> {
    AOM2_FILES.iter().map(|f| aom2_dir.join(f)).collect()
}

/// The AOM2 model-form closure (`Archetype.xsd` and its includes).
///
/// Its declared root `ARCHETYPE` is abstract and nothing derives from it, so
/// the entry points are typed to `AUTHORED_ARCHETYPE`.
pub(crate) const AOM2_MODEL_FILES: &[&str] = &[
    "Archetype.xsd",
    "ArchetypeCommon.xsd",
    "Rules.xsd",
    "Resource.xsd",
    "BaseTypes.xsd",
];

/// Resolve the AOM2 model-form schema file paths under the bundle's `AOM2/` dir.
#[must_use]
pub(crate) fn aom2_model_files(aom2_dir: &Path) -> Vec<std::path::PathBuf> {
    AOM2_MODEL_FILES.iter().map(|f| aom2_dir.join(f)).collect()
}

// The two ITS-XML lineages differ only in target namespace.
