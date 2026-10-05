// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The path catalog the Query Builder navigates.
//!
//! A slim, serializable tree distilled BFF-side from the Web Template (built
//! from the CDR's OPT with
//! `openehr_sdt::flat::webtemplate::builder::build_web_template` — the same code the
//! CDR serves `application/openehr.wt+json` with) and shipped to the browser.
//! Node shape per `docs/specs/openehr/ITS-REST/docs/simplified_formats/
//! master04-basic_concepts.adoc` §"Web Template Metadata".

use serde::{Deserialize, Serialize};

/// One selectable coded option (a `DV_CODED_TEXT` code or a `DV_ORDINAL`
/// step) for the criteria widgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeOption {
    /// The code string (`at0037`, a SNOMED code, …).
    pub code: String,
    /// Display label (falls back to the code).
    pub label: String,
    /// The ordinal value when the option belongs to a `DV_ORDINAL`.
    pub ordinal: Option<i32>,
}

/// One node of the builder's path tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogNode {
    /// Display label (localized name → name → json id).
    pub label: String,
    /// The RM type (`DV_QUANTITY`, `OBSERVATION`, …) — full names, no
    /// compaction.
    pub rm_type: String,
    /// COMPOSITION-relative archetype path (the WT `aqlPath`; for a
    /// promoted leaf this IS the `DATA_VALUE` node's path).
    pub aql_path: String,
    /// The archetype node id (`at0004` / an archetype HRID), empty if none.
    pub node_id: String,
    /// Whether criteria/columns can target this node (a `DATA_VALUE` leaf).
    pub selectable: bool,
    /// Coded/ordinal options for the typed widgets (empty otherwise).
    pub code_options: Vec<CodeOption>,
    /// `DV_QUANTITY` unit options (empty otherwise).
    pub unit_options: Vec<String>,
    /// Child nodes.
    pub children: Vec<CatalogNode>,
}

/// Distill the Web Template tree into the catalog (BFF-side).
#[cfg(feature = "ssr")]
#[must_use]
pub fn from_web_template(wt: &openehr_sdt::flat::webtemplate::model::WebTemplate) -> CatalogNode {
    node(&wt.tree)
}

#[cfg(feature = "ssr")]
fn node(wt: &openehr_sdt::flat::webtemplate::model::WebTemplateNode) -> CatalogNode {
    use openehr_sdt::flat::webtemplate::model::WebTemplateInputType;

    let label = wt
        .localized_name
        .clone()
        .or_else(|| wt.name.clone())
        .unwrap_or_else(|| wt.id.clone());

    let mut code_options = Vec::new();
    let mut unit_options = Vec::new();
    for input in &wt.inputs {
        match (&input.input_type, input.suffix.as_deref()) {
            // DV_QUANTITY property/unit selector.
            (WebTemplateInputType::Text | WebTemplateInputType::CodedText, Some("unit")) => {
                unit_options.extend(input.list.iter().map(|v| v.value.clone()));
            }
            // Coded / ordinal value sets.
            (WebTemplateInputType::CodedText, _) => {
                code_options.extend(input.list.iter().map(|v| CodeOption {
                    code: v.value.clone(),
                    label: v.label.clone().unwrap_or_else(|| v.value.clone()),
                    ordinal: v.ordinal,
                }));
            }
            _ => {}
        }
    }

    CatalogNode {
        label,
        rm_type: wt.rm_type.clone(),
        aql_path: wt.aql_path.clone(),
        node_id: wt.node_id.clone().unwrap_or_default(),
        selectable: wt.rm_type.starts_with("DV_"),
        code_options,
        unit_options,
        children: wt.children.iter().map(node).collect(),
    }
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use openehr_sdt::flat::webtemplate::model::{
        WebTemplateCodedValue, WebTemplateInput, WebTemplateInputType, WebTemplateNode,
    };

    use crate::builder::catalog::from_web_template;

    #[test]
    fn catalog_extracts_labels_selectability_units_and_codes() {
        let mut quantity = WebTemplateNode::new(
            "DV_QUANTITY".to_owned(),
            "/content[openEHR-EHR-OBSERVATION.body_temperature.v2]/data[at0002]/events[at0003]/data[at0001]/items[at0004]/value".to_owned(),
        );
        quantity.id = "temperature".to_owned();
        quantity.localized_name = Some("Temperature".to_owned());
        let mut unit_input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("unit"));
        unit_input.list.push(WebTemplateCodedValue::new("°C", None));
        unit_input.list.push(WebTemplateCodedValue::new("°F", None));
        quantity.inputs.push(unit_input);

        let mut coded = WebTemplateNode::new(
            "DV_CODED_TEXT".to_owned(),
            "/context/other_context[at0001]/items[at0005]/value".to_owned(),
        );
        coded.id = "position".to_owned();
        let mut code_input = WebTemplateInput::new(WebTemplateInputType::CodedText, Some("code"));
        code_input.list.push(WebTemplateCodedValue::new(
            "at0037",
            Some("Sitting".to_owned()),
        ));
        coded.inputs.push(code_input);

        let mut obs = WebTemplateNode::new(
            "OBSERVATION".to_owned(),
            "/content[openEHR-EHR-OBSERVATION.body_temperature.v2]".to_owned(),
        );
        obs.id = "body_temperature".to_owned();
        obs.children.push(quantity);
        obs.children.push(coded);

        let mut root = WebTemplateNode::new("COMPOSITION".to_owned(), String::new());
        root.id = "vitals".to_owned();
        root.children.push(obs);

        let wt = openehr_sdt::flat::webtemplate::model::WebTemplate {
            template_id: "vitals.v1".to_owned(),
            sem_ver: None,
            version: "2.3".to_owned(),
            default_language: "en".to_owned(),
            languages: vec!["en".to_owned()],
            tree: root,
            other_details: indexmap::IndexMap::new(),
        };

        let catalog = from_web_template(&wt);
        assert!(!catalog.selectable);
        let obs = &catalog.children[0];
        assert_eq!(obs.rm_type, "OBSERVATION");
        let temp = &obs.children[0];
        assert!(temp.selectable);
        assert_eq!(temp.label, "Temperature");
        assert_eq!(temp.unit_options, vec!["°C", "°F"]);
        let pos = &obs.children[1];
        assert_eq!(pos.code_options.len(), 1);
        assert_eq!(pos.code_options[0].label, "Sitting");
    }
}
