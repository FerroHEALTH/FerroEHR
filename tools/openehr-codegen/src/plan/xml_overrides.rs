// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `emit-xml` decision data: where the generated canonical-XML codec departs
//! from the plain match of BMM fields to the vendored ITS-XML schemas, as
//! checked-in const tables, each entry with its spec citation.

// ─────────────────────────────────────────────────────────────────────────────
// emit-xml BMM-only field allowlist (RM/BASE model vs vendored ITS-XML skew)
// ─────────────────────────────────────────────────────────────────────────────

/// One allowlisted BMM-only field: the owning concrete type's spec name, the
/// field's wire name, the governing spec-version skew, and a one-line reason
/// bucket.
pub(crate) struct XmlBmmOnlyField {
    /// The owning concrete type's spec name.
    pub spec: &'static str,
    /// The field's openEHR wire (property) name.
    pub wire_name: &'static str,
    /// Why the field is absent from the vendored XSD and safe to append — the
    /// concrete RM/BASE-model-vs-ITS-XML-XSD version delta.
    pub citation: &'static str,
    /// One-line reason bucket.
    pub reason: &'static str,
}

/// Allowlist of BMM fields that have **no** matching element or attribute in any
/// vendored ITS-XML XSD (the RM/BASE model outran the vendored ITS-XML schemas).
/// Each accepted `(spec, wire_name)` pair is emitted as a **deterministic
/// trailing element** in BMM field order (so it is never silently dropped), and
/// carries the spec delta that justifies it. Any XSD-covered struct with a BMM
/// field NOT on this list fails codegen (the `check_bmm_field_coverage` guard in
/// `render::emit_xml`), forcing an explicit decision instead of a silent drop.
///
/// The governing spec citation for every entry is the pinned-version delta: the
/// emitted models are RM 1.1.0 and 1.2.0 over BASE 1.2.0 and 1.3.0, while the
/// vendored canonical-XML schemas are ITS-XML 1.0.2 (namespace `.../v1`) and, for
/// the EHR/demographic/extract closure, ITS-XML 2.0.0 (RM 1.1.0). Where the model
/// added or renamed a field after those XSDs were cut, no XSD slot exists.
///
/// Empty is the desired state: it means every RM-1.2.0 BMM field of an
/// XSD-covered type maps to a vendored-XSD slot.
pub(crate) const XML_BMM_ONLY_ALLOWLIST: &[XmlBmmOnlyField] = &[
    // ── BASE 1.3.0 resource-class growth (ITS-XML 1.0.2 Resource.xsd predates it) ──
    XmlBmmOnlyField {
        spec: "AUTHORED_RESOURCE",
        wire_name: "uid",
        citation: "BASE 1.3.0 added AUTHORED_RESOURCE.uid (HIER_OBJECT_ID); absent from ITS-XML 1.0.2 Resource.xsd.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "AUTHORED_RESOURCE",
        wire_name: "annotations",
        citation: "BASE 1.3.0 added AUTHORED_RESOURCE.annotations (RESOURCE_ANNOTATIONS); absent from ITS-XML 1.0.2 Resource.xsd.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "title",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.title; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "original_namespace",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.original_namespace; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "original_publisher",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.original_publisher; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "custodian_namespace",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.custodian_namespace; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "custodian_organisation",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.custodian_organisation; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "copyright",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.copyright; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "licence",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.licence; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "ip_acknowledgements",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.ip_acknowledgements; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "references",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.references; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "RESOURCE_DESCRIPTION",
        wire_name: "conversion_details",
        citation: "BASE 1.3.0 RESOURCE_DESCRIPTION.conversion_details; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "TRANSLATION_DETAILS",
        wire_name: "version_last_translated",
        citation: "BASE 1.3.0 TRANSLATION_DETAILS.version_last_translated; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "TRANSLATION_DETAILS",
        wire_name: "other_contributors",
        citation: "BASE 1.3.0 TRANSLATION_DETAILS.other_contributors; ITS-XML 1.0.2 Resource.xsd lacks it.",
        reason: "BASE 1.3.0 resource-class growth vs ITS-XML 1.0.2.",
    },
    // ── BASE 1.3.0 CODE_PHRASE.preferred_term ──
    XmlBmmOnlyField {
        spec: "CODE_PHRASE",
        wire_name: "preferred_term",
        citation: "BASE 1.3.0 added CODE_PHRASE.preferred_term; ITS-XML 1.0.2 BaseTypes.xsd CODE_PHRASE has terminology_id + code_string only.",
        reason: "BASE 1.3.0 field addition vs ITS-XML 1.0.2.",
    },
    // ── RM 1.2.0 ENTRY.work_flow_id → workflow_id rename (all ENTRY subtypes) ──
    XmlBmmOnlyField {
        spec: "ACTION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id. Canonical JSON corpus uses workflow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "ADMIN_ENTRY",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "EVALUATION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "INSTRUCTION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "OBSERVATION",
        wire_name: "workflow_id",
        citation: "RM 1.2.0 renamed ENTRY.work_flow_id → workflow_id; ITS-XML 1.0.2 Content.xsd still declares work_flow_id.",
        reason: "RM 1.2.0 ENTRY.work_flow_id → workflow_id rename vs ITS-XML 1.0.2.",
    },
    // ── RM 1.2.0 data-type / data-structure additions ──
    XmlBmmOnlyField {
        spec: "DV_QUANTITY",
        wire_name: "units_system",
        citation: "RM 1.2.0 added DV_QUANTITY.units_system; absent from ITS-XML 1.0.2 BaseTypes.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "DV_QUANTITY",
        wire_name: "units_display_name",
        citation: "RM 1.2.0 added DV_QUANTITY.units_display_name; absent from ITS-XML 1.0.2 BaseTypes.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "ELEMENT",
        wire_name: "null_reason",
        citation: "RM 1.2.0 added ELEMENT.null_reason; ITS-XML 1.0.2 Structure.xsd ELEMENT has value + null_flavour only.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "ISM_TRANSITION",
        wire_name: "reason",
        citation: "RM 1.2.0 added ISM_TRANSITION.reason (List<DV_TEXT>); ITS-XML 1.0.2 Content.xsd has current_state/transition/careflow_step only.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "FEEDER_AUDIT_DETAILS",
        wire_name: "other_details",
        citation: "RM 1.2.0 added FEEDER_AUDIT_DETAILS.other_details (ITEM_STRUCTURE); absent from ITS-XML 1.0.2 Common.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "FOLDER",
        wire_name: "details",
        citation: "RM 1.2.0 added FOLDER.details (ITEM_STRUCTURE); absent from ITS-XML 1.0.2 Composition.xsd.",
        reason: "RM 1.2.0 field addition vs ITS-XML 1.0.2.",
    },
    XmlBmmOnlyField {
        spec: "EHR",
        wire_name: "tags",
        citation: "RM 1.2.0 added EHR.tags; absent from ITS-XML 2.0.0 (RM 1.1.0) Ehr.xsd, the vendored EHR shape.",
        reason: "RM 1.2.0 field addition vs ITS-XML 2.0.0 (RM 1.1.0).",
    },
    // ── RM 1.2.0 EhrExtract includes_* → include_* renames ──
    XmlBmmOnlyField {
        spec: "EXTRACT_SPEC",
        wire_name: "include_multimedia",
        citation: "RM 1.2.0 renamed EXTRACT_SPEC.includes_multimedia → include_multimedia; ITS-XML 2.0.0 EhrExtract.xsd still declares includes_multimedia.",
        reason: "RM 1.2.0 EhrExtract includes_* → include_* rename vs ITS-XML 2.0.0.",
    },
    XmlBmmOnlyField {
        spec: "EXTRACT_VERSION_SPEC",
        wire_name: "include_revision_history",
        citation: "RM 1.2.0 renamed EXTRACT_VERSION_SPEC.includes_revision_history → include_revision_history; ITS-XML 2.0.0 EhrExtract.xsd still declares includes_revision_history.",
        reason: "RM 1.2.0 EhrExtract includes_* → include_* rename vs ITS-XML 2.0.0.",
    },
    XmlBmmOnlyField {
        spec: "EXTRACT_VERSION_SPEC",
        wire_name: "include_data",
        citation: "RM 1.2.0 renamed EXTRACT_VERSION_SPEC.includes_data → include_data; ITS-XML 2.0.0 EhrExtract.xsd still declares includes_data.",
        reason: "RM 1.2.0 EhrExtract includes_* → include_* rename vs ITS-XML 2.0.0.",
    },
    // ── VERSIONED_OBJECT base fields on the VERSIONED_* container types ──
    // uid/owner_id/time_created come from VERSIONED_OBJECT, defined ONLY in the
    // v2 Common.xsd (RM 1.1.0), which the emit-xml input deliberately does not
    // merge: v1 Version.xsd omits VERSIONED_OBJECT, and pulling in v2 Common.xsd
    // would re-shape the served VERSION-family types. These container types are
    // not served as canonical XML (406 at the REST edge); the base fields are
    // appended in canonical VERSIONED_OBJECT order so nothing is lost.
    XmlBmmOnlyField {
        spec: "VERSIONED_COMPOSITION",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd (see module note).",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_COMPOSITION",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_COMPOSITION",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_ACCESS",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_ACCESS",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_ACCESS",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_STATUS",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_STATUS",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_EHR_STATUS",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_PARTY",
        wire_name: "uid",
        citation: "VERSIONED_OBJECT.uid; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_PARTY",
        wire_name: "owner_id",
        citation: "VERSIONED_OBJECT.owner_id; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
    XmlBmmOnlyField {
        spec: "VERSIONED_PARTY",
        wire_name: "time_created",
        citation: "VERSIONED_OBJECT.time_created; base defined only in un-merged v2 Common.xsd.",
        reason: "VERSIONED_OBJECT base field from the un-merged v2 Common.xsd.",
    },
];

/// Is `(spec, wire_name)` an accepted BMM-only field (on the allowlist)?
pub(crate) fn xml_bmm_only_allowed(spec: &str, wire_name: &str) -> bool {
    XML_BMM_ONLY_ALLOWLIST
        .iter()
        .any(|e| e.spec == spec && e.wire_name == wire_name)
}

// ─────────────────────────────────────────────────────────────────────────────
// emit-xml element names the ITS-XML schemas spell apart from the BMM
// ─────────────────────────────────────────────────────────────────────────────

/// One BMM property whose canonical-XML element the ITS-XML schema names
/// differently.
pub(crate) struct XmlWireRename {
    /// The class declaring the property.
    pub owner: &'static str,
    /// The property's BMM name.
    pub bmm_name: &'static str,
    /// The element name the ITS-XML schema declares.
    pub element: &'static str,
}

/// The BMM properties written and read under the ITS-XML element name.
pub(crate) const XML_WIRE_RENAMES: &[XmlWireRename] = &[
    // NOTE: ITS-XML 1.0.2 `Resource.xsd` declares `accreditation`, which the RM 1.1.0
    // BMM misspells `accreditaton` (RM 1.2.0 `translation_details.adoc` §Attributes).
    XmlWireRename {
        owner: "TRANSLATION_DETAILS",
        bmm_name: "accreditaton",
        element: "accreditation",
    },
];

/// The ITS-XML element name of `owner.bmm_name`, when it differs from the BMM
/// name.
pub(crate) fn xml_wire_rename(owner: &str, bmm_name: &str) -> Option<&'static str> {
    XML_WIRE_RENAMES
        .iter()
        .find(|e| e.owner == owner && e.bmm_name == bmm_name)
        .map(|e| e.element)
}

// ─────────────────────────────────────────────────────────────────────────────
// emit-xml classes no vendored ITS-XML schema declares
// ─────────────────────────────────────────────────────────────────────────────

/// One class that no vendored ITS-XML schema declares, written in the shape of
/// its nearest XSD-declared BMM ancestor.
pub(crate) struct XmlAncestorShape {
    /// The class's spec name.
    pub spec: &'static str,
    /// The nearest BMM ancestor an ITS-XML schema declares, whose attribute and
    /// element split the class takes; the class's own fields follow in BMM
    /// order.
    pub ancestor: &'static str,
}

/// The classes the canonical-XML codec writes in their nearest XSD-declared
/// ancestor's shape, so `archetype_node_id` stays the attribute `LOCATABLE`
/// requires. Every other class no XSD declares still fails the generation
/// (the LOCATABLE-attribute guard in `render::emit_xml`).
// NOTE: no ITS-XML schema governs these, our own design: ITS-XML RM Release-1.1.0
// declares neither RM 1.1.0 BMM class (`composition.content.view`), reported in #4843.
pub(crate) const XML_ANCESTOR_SHAPES: &[XmlAncestorShape] = &[
    XmlAncestorShape {
        spec: "CITATION",
        ancestor: "CONTENT_ITEM",
    },
    XmlAncestorShape {
        spec: "VIEW_SECTION",
        ancestor: "CONTENT_ITEM",
    },
];

/// The XSD-declared ancestor whose shape `spec` takes, when `spec` is on
/// [`XML_ANCESTOR_SHAPES`].
pub(crate) fn xml_ancestor_shape(spec: &str) -> Option<&'static str> {
    XML_ANCESTOR_SHAPES
        .iter()
        .find(|e| e.spec == spec)
        .map(|e| e.ancestor)
}

// ─────────────────────────────────────────────────────────────────────────────
// emit-xml tool-emitted extension elements (outside every vendored ITS-XML XSD)
// ─────────────────────────────────────────────────────────────────────────────

/// One element modelling tools write into a type's content although no vendored
/// ITS-XML schema declares it there.
pub(crate) struct XmlToolExtension {
    /// The owning concrete type's spec name.
    pub spec: &'static str,
    /// The element's name.
    pub element: &'static str,
    /// Where the element is seen, and why it is read rather than refused.
    pub citation: &'static str,
}

/// The tool-emitted extension elements the generated XML readers accept, each
/// only with the boolean value `false`: a `true` would change the constraint's
/// meaning in a way no schema defines, so it is refused
/// (`XmlError::UnsupportedExtension`). Every other element outside a type's
/// closed `xs:sequence` is refused (XML Schema 1.1 Part 1 §3.4.4.2).
pub(crate) const XML_TOOL_EXTENSIONS: &[XmlToolExtension] = &[
    XmlToolExtension {
        spec: "C_SINGLE_ATTRIBUTE",
        element: "match_negated",
        citation: "Not in ITS-XML 1.0.2 Archetype.xsd/Template.xsd nor ITS-XML 2.0.0 \
                   AM/Release-1.4/Template.xsd; carried by the CNF Robot valid_templates \
                   cache/case.opt, cache/case2.opt and all_types/ehrn_vital_signs.v2.opt.",
    },
    XmlToolExtension {
        spec: "C_MULTIPLE_ATTRIBUTE",
        element: "match_negated",
        citation: "As for C_SINGLE_ATTRIBUTE: the tools write it on both C_ATTRIBUTE subtypes.",
    },
];

/// The tool-emitted extension elements accepted under `spec`, each written into
/// the generated reader with its citation as a `// NOTE:`.
pub(crate) fn xml_tool_extensions(
    spec: &str,
) -> impl Iterator<Item = &'static XmlToolExtension> + '_ {
    XML_TOOL_EXTENSIONS.iter().filter(move |e| e.spec == spec)
}

/// One model `String` field the canonical-XML wire carries in the `OBJECT_ID`
/// shape (`<field><value>text</value></field>`): the owning type's spec name,
/// the field's wire name, and the schema delta that makes it so.
pub(crate) struct XmlValueWrapped {
    /// The owning concrete type's spec name.
    pub spec: &'static str,
    /// The field's wire name.
    pub wire_name: &'static str,
    /// The model-versus-schema delta.
    pub citation: &'static str,
}

/// The model `String` fields written and read in the `OBJECT_ID` shape.
pub(crate) const XML_VALUE_WRAPPED: &[XmlValueWrapped] = &[XmlValueWrapped {
    spec: "Terminology_code",
    wire_name: "terminology_id",
    citation: "BASE 1.3.0 TERMINOLOGY_CODE.terminology_id is a String; ITS-XML 1.0.2 \
               Resource.xsd types the slot CODE_PHRASE, whose terminology_id is a \
               TERMINOLOGY_ID with one value element (BaseTypes.xsd).",
}];

/// Whether `(spec, wire_name)` is written and read in the `OBJECT_ID` shape;
/// the citation when it is.
pub(crate) fn xml_value_wrapped(spec: &str, wire_name: &str) -> Option<&'static str> {
    XML_VALUE_WRAPPED
        .iter()
        .find(|e| e.spec == spec && e.wire_name == wire_name)
        .map(|e| e.citation)
}
