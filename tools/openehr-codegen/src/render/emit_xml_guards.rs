// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The guards `emit-xml` runs over every emitted type; generation fails with
//! the complete list.

use crate::load::xsd::XsdModel;
use crate::plan::XmlType;
use crate::plan::xml_overrides::{XML_ANCESTOR_SHAPES, xml_ancestor_shape, xml_bmm_only_allowed};
use crate::render::emit_xml::{attr_names, bmm_only_fields};

/// What the guards observed over the emitted types.
#[derive(Debug, Default)]
pub(crate) struct Guards {
    /// LOCATABLE subtypes whose `archetype_node_id` would be an element.
    locatable: Vec<String>,
    /// BMM fields with no XSD slot and no allowlist entry.
    bmm_only: Vec<String>,
    /// Allowlisted BMM-only fields appended as trailing elements.
    appended: Vec<String>,
}

impl Guards {
    /// Records what `ty` trips.
    pub(crate) fn observe(&mut self, ty: &XmlType, xsd: &XsdModel) {
        check_locatable_attr(ty, xsd, &mut self.locatable);
        check_bmm_field_coverage(ty, xsd, &mut self.bmm_only);
        if let XmlType::Struct { spec, fields, .. } = ty {
            for f in bmm_only_fields(spec, fields, xsd) {
                self.appended.push(format!("{spec}.{}", f.wire_name));
            }
        }
    }

    /// Reports the appended fields and fails on any violation.
    ///
    /// # Errors
    /// Names every violation of the ancestor-shape, LOCATABLE-attribute and
    /// BMM-field-coverage guards.
    pub(crate) fn verdict(self, xsd: &XsdModel) -> Result<(), String> {
        if !self.appended.is_empty() {
            eprintln!(
                "emit-xml: {} allowlisted BMM-only field(s) appended as trailing \
                 canonical-XML elements (RM/BASE model vs vendored ITS-XML skew): {}",
                self.appended.len(),
                self.appended.join(", ")
            );
        }
        let stale_shapes: Vec<String> = XML_ANCESTOR_SHAPES
            .iter()
            .filter(|e| xsd.types.contains_key(e.spec) || !xsd.types.contains_key(e.ancestor))
            .map(|e| format!("{} -> {}", e.spec, e.ancestor))
            .collect();
        if !stale_shapes.is_empty() {
            return Err(format!(
                "emit-xml ancestor-shape guard: an XML_ANCESTOR_SHAPES entry \
                 (plan/xml_overrides.rs) names a class the XSD input now declares, or an \
                 ancestor it does not: {}",
                stale_shapes.join(", ")
            ));
        }
        if !self.locatable.is_empty() {
            return Err(format!(
                "emit-xml LOCATABLE-attribute guard: {} LOCATABLE subtype(s) have no XSD closure, \
                 so `archetype_node_id` would serialize as a child element instead of the \
                 required XML attribute: {}. Extend the XSD input \
                 (tools/openehr-codegen/src/load/xsd.rs) so these types reach `LOCATABLE`.",
                self.locatable.len(),
                self.locatable.join(", ")
            ));
        }
        if !self.bmm_only.is_empty() {
            return Err(format!(
                "emit-xml BMM-field-coverage guard: {} BMM field(s) of XSD-covered type(s) have \
                 no matching XSD element/attribute, so the XSD-driven `ToXml` would silently \
                 drop them while the BMM-driven `FromXml` still reads them (an undetectable \
                 JSON/XML divergence). Either extend the driving XSD input \
                 (tools/openehr-codegen/src/load/xsd.rs) so the field gets a deterministic slot, \
                 or add it to `XML_BMM_ONLY_ALLOWLIST` in plan/xml_overrides.rs (with a \
                 spec-delta citation) to have it appended as a trailing canonical-XML element. \
                 Fields: {}.",
                self.bmm_only.len(),
                self.bmm_only.join(", ")
            ));
        }
        Ok(())
    }
}

/// Records an `archetype_node_id` field the XSD does not classify as an
/// attribute, which LOCATABLE requires (`use="required"`).
fn check_locatable_attr(ty: &XmlType, xsd: &XsdModel, violations: &mut Vec<String>) {
    if let XmlType::Struct { spec, fields, .. } = ty {
        let has_archetype_node_id = fields.iter().any(|f| f.wire_name == "archetype_node_id");
        if has_archetype_node_id && !attr_names(spec, xsd).contains("archetype_node_id") {
            violations.push(spec.clone());
        }
    }
}

/// Records every BMM field of an XSD-covered struct with no XSD slot and no
/// [`crate::plan::xml_overrides::XML_BMM_ONLY_ALLOWLIST`] entry.
fn check_bmm_field_coverage(ty: &XmlType, xsd: &XsdModel, violations: &mut Vec<String>) {
    if let XmlType::Struct { spec, fields, .. } = ty {
        // An ancestor-shaped class appends its own fields by the override's design.
        if !xsd.types.contains_key(spec.as_str()) && xml_ancestor_shape(spec).is_some() {
            return;
        }
        for f in bmm_only_fields(spec, fields, xsd) {
            if !xml_bmm_only_allowed(spec, &f.wire_name) {
                violations.push(format!("{spec}.{}", f.wire_name));
            }
        }
    }
}
