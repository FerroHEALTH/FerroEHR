// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The FHIR-connector reverse transform: canonical COMPOSITION to FHIR
//! resource, shared by the read façade and the outbound emitter.
//!
//! No openEHR spec governs this — our own design/extension.
//!
//! The exact inverse of [`build_flat`](super::mapping::build_flat): the
//! COMPOSITION is flattened through
//! [`composition_to_flat`](openehr_sdt::flat::convert::composition_to_flat) to
//! the same FLAT map
//! [`composition_from_flat`](openehr_sdt::flat::convert::composition_from_flat)
//! consumes, so the leaf keys are byte-identical to what an entry wrote inbound;
//! each mapping entry then reads its leaves back out and writes them to its
//! `FHIRPath`-lite target. `code_map` is applied in reverse.
//!
//! A `constant` entry is not reversed: it injected a fixed openEHR leaf with no
//! FHIR source, so round-trip fidelity is defined over the FHIR-sourced fields
//! only. The `subject` is reconstructed from the owning EHR's subject id, which
//! the façade or emitter supplies, with `strip_prefix` re-applied so the
//! reference matches the inbound form.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 6): FHIR resources are an external standard \
              with no RM type (typed-FHIR evaluation tracked separately)"
)]

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::mapping::{FhirMapError, FhirMappingDefinition, MappingEntry, Transform, parse_segment};

/// Reverse of [`build_flat`](super::mapping::build_flat): builds the FHIR
/// resource for a COMPOSITION under a mapping definition.
///
/// `subject_id` is the owning EHR's external subject id (placed back at the
/// mapping's `subject.reference_path`, `strip_prefix` re-applied); `None`
/// omits the subject.
/// # Errors
/// Returns [`FhirMapError`] when the mapping definition cannot be applied
/// in reverse over the stored FLAT projection.
pub fn to_fhir(
    resource_type: &str,
    composition: &Value,
    wt: &openehr_sdt::flat::webtemplate::model::WebTemplate,
    def: &FhirMappingDefinition,
    subject_id: Option<&str>,
) -> Result<Value, FhirMapError> {
    let flat = openehr_sdt::flat::convert::composition_to_flat(composition, wt)
        .map_err(|e| FhirMapError::ReverseFailed(e.to_string(), e))?;
    let mut resource = json!({ "resourceType": resource_type });
    if let Some(sid) = subject_id {
        let reference = match &def.subject.strip_prefix {
            Some(prefix) => format!("{prefix}{sid}"),
            None => sid.to_owned(),
        };
        set_at(
            &mut resource,
            &def.subject.reference_path,
            Value::String(reference),
        );
    }
    for entry in &def.entries {
        // A constant has no FHIR source (see the module NOTE).
        if entry.constant.is_some() {
            continue;
        }
        reverse_entry(&flat, entry, &mut resource);
    }
    Ok(resource)
}

/// Reverse one entry: read its FLAT leaf(s) and write them to the FHIRPath-lite
/// target(s). Absent leaves are simply not written (a partially-populated
/// COMPOSITION yields a partially-populated resource — the inverse of inbound's
/// skip-when-absent).
fn reverse_entry(flat: &impl FlatLookup, entry: &MappingEntry, resource: &mut Value) {
    let Some(fhir_path) = entry.fhir_path.as_deref() else {
        return;
    };
    match &entry.transform {
        Transform::Text | Transform::Date => {
            if let Some(v) = flat.lookup(&entry.openehr_path) {
                set_at(resource, fhir_path, v.clone());
            }
        }
        Transform::Quantity { unit_path, .. } => {
            if let Some(v) = flat.lookup(&format!("{}|magnitude", entry.openehr_path)) {
                set_at(resource, fhir_path, v.clone());
            }
            // A literal `unit` had no FHIR source, so only a `unit_path` is
            // reversed (the inverse of inbound: literal wins over path there).
            if let Some(up) = unit_path
                && let Some(u) = flat.lookup(&format!("{}|unit", entry.openehr_path))
            {
                set_at(resource, up, u.clone());
            }
        }
        Transform::Coded {
            system_path,
            display_path,
            translate: _,
        } => reverse_coded(
            flat,
            entry,
            fhir_path,
            system_path.as_deref(),
            display_path.as_deref(),
            resource,
        ),
    }
}

/// Reverses a coded leaf into its FHIR coding fields.
///
/// NOTE: no openEHR spec governs FHIR conversion — our own design: a
/// `translate` entry reverses to the stored (translated) coding, whose
/// `|code`/`|terminology` pair is internally consistent.
fn reverse_coded(
    flat: &impl FlatLookup,
    entry: &MappingEntry,
    fhir_path: &str,
    system_path: Option<&str>,
    display_path: Option<&str>,
    resource: &mut Value,
) {
    if let Some(code) = flat.lookup(&format!("{}|code", entry.openehr_path)) {
        set_at(resource, fhir_path, code.clone());
    }
    if let Some(sp) = system_path
        && let Some(term) = flat
            .lookup(&format!("{}|terminology", entry.openehr_path))
            .and_then(Value::as_str)
        && let Some(system) = reverse_code_map(&entry.code_map, term)
    {
        set_at(resource, sp, Value::String(system));
    }
    if let Some(dp) = display_path
        && let Some(display) = flat.lookup(&format!("{}|value", entry.openehr_path))
    {
        set_at(resource, dp, display.clone());
    }
}

/// Reverse `code_map` (FHIR system URL → openEHR `terminology_id`): given a
/// `terminology_id`, recover the FHIR system that mapped to it. The `*`
/// wildcard is skipped (it is lossy — many systems fold to one `terminology_id`
/// through it, so the original cannot be recovered); `BTreeMap` iteration order
/// makes the choice deterministic when several systems share a `terminology_id`.
fn reverse_code_map(code_map: &BTreeMap<String, String>, terminology: &str) -> Option<String> {
    code_map
        .iter()
        .find(|(system, term)| system.as_str() != "*" && term.as_str() == terminology)
        .map(|(system, _)| system.clone())
}

/// A minimal FLAT-map lookup seam so [`reverse_entry`] can be exercised against
/// both
/// [`composition_to_flat`](openehr_sdt::flat::convert::composition_to_flat)'s map and
/// a plain map in unit tests without naming the crate-private `FlatMap` alias.
trait FlatLookup {
    /// The value stored at the exact FLAT key, if any.
    fn lookup(&self, key: &str) -> Option<&Value>;
}

impl FlatLookup for indexmap::IndexMap<String, Value> {
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

impl FlatLookup for Map<String, Value> {
    fn lookup(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

/// Write `value` into `root` at a **`FHIRPath`-lite** path (the same grammar
/// [`resolve`](super::mapping::resolve) reads), materialising intermediate
/// objects/arrays as it descends — the write-side inverse of `resolve`. A
/// malformed segment aborts silently (the entry simply does not contribute),
/// matching inbound's tolerant skip.
fn set_at(root: &mut Value, path: &str, value: Value) {
    let segments: Option<Vec<(&str, Option<usize>)>> = path.split('.').map(parse_segment).collect();
    if let Some(segments) = segments {
        place(root, &segments, value);
    }
}

/// Descend `segments` from `cur`, creating objects/arrays as needed, and write
/// `value` at the leaf.
fn place(cur: &mut Value, segments: &[(&str, Option<usize>)], value: Value) {
    let Some(((name, index), rest)) = segments.split_first() else {
        *cur = value;
        return;
    };
    // A named segment selects (creating if absent) an object field.
    let slot: &mut Value = if name.is_empty() {
        cur
    } else {
        if !cur.is_object() {
            *cur = Value::Object(Map::new());
        }
        match cur.as_object_mut() {
            Some(obj) => obj.entry((*name).to_owned()).or_insert(Value::Null),
            None => return,
        }
    };
    match index {
        // An indexed segment selects (creating + null-padding) an array element.
        Some(i) => {
            if !slot.is_array() {
                *slot = Value::Array(Vec::new());
            }
            let Some(arr) = slot.as_array_mut() else {
                return;
            };
            while arr.len() <= *i {
                arr.push(Value::Null);
            }
            // The loop above null-padded up to `*i`, so the element exists;
            // fetched rather than indexed, so the padding is the only thing
            // that has to stay correct.
            if let Some(elem) = arr.get_mut(*i) {
                place(elem, rest, value);
            }
        }
        None => place(slot, rest, value),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use openehr_its::opt14;
    use openehr_sdt::flat::convert::composition_from_flat;
    use openehr_sdt::flat::webtemplate::builder::build_web_template;
    use openehr_sdt::flat::webtemplate::model::WebTemplate;
    use serde_json::json;

    use super::super::mapping::{CodeTranslations, build_flat};
    use super::*;

    fn def(v: Value) -> FhirMappingDefinition {
        serde_json::from_value(v).expect("valid mapping definition")
    }

    /// The same corpus template the inbound end-to-end test uses (see
    /// `mapping::tests`): a small single-OBSERVATION blood-pressure OPT.
    fn bp_web_template() -> WebTemplate {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../corpus/fixtures/service/knowledge/opt/ehrbase_blood_pressure_simple.de.v0.opt",
        );
        let xml = std::fs::read_to_string(path).expect("read opt");
        let opt = opt14::from_xml(&xml).expect("parse opt");
        build_web_template(&opt).expect("build wt")
    }

    #[test]
    fn set_at_builds_nested_objects_and_arrays() {
        let mut r = json!({ "resourceType": "Observation" });
        set_at(&mut r, "component[1].valueQuantity.value", json!(80));
        set_at(&mut r, "component[0].valueQuantity.value", json!(120));
        set_at(&mut r, "subject.reference", json!("Patient/p-1"));
        // Array is null-padded to the highest index then filled.
        assert_eq!(r["component"][0]["valueQuantity"]["value"], json!(120));
        assert_eq!(r["component"][1]["valueQuantity"]["value"], json!(80));
        assert_eq!(r["subject"]["reference"], json!("Patient/p-1"));
    }

    #[test]
    fn reverse_code_map_recovers_system_and_skips_wildcard() {
        let mut m = BTreeMap::new();
        m.insert("http://snomed.info/sct".to_owned(), "SNOMED-CT".to_owned());
        m.insert("*".to_owned(), "SNOMED-CT".to_owned());
        assert_eq!(
            reverse_code_map(&m, "SNOMED-CT").as_deref(),
            Some("http://snomed.info/sct")
        );
        // A terminology only reachable via the lossy `*` fallback is not
        // recovered (the original system is unknowable).
        let mut only_wild = BTreeMap::new();
        only_wild.insert("*".to_owned(), "local".to_owned());
        assert_eq!(reverse_code_map(&only_wild, "local"), None);
    }

    #[test]
    fn reverse_entry_coded_places_code_system_and_display() {
        let entry: MappingEntry = serde_json::from_value(json!({
            "openehr_path": "x/problem",
            "fhir_path": "code.coding[0].code",
            "transform": { "kind": "coded", "system_path": "code.coding[0].system", "display_path": "code.coding[0].display" },
            "code_map": { "http://snomed.info/sct": "SNOMED-CT" }
        }))
        .expect("entry");
        let mut flat = Map::new();
        flat.insert("x/problem|code".to_owned(), json!("73211009"));
        flat.insert("x/problem|terminology".to_owned(), json!("SNOMED-CT"));
        flat.insert("x/problem|value".to_owned(), json!("Diabetes mellitus"));
        let mut r = json!({ "resourceType": "Condition" });
        reverse_entry(&flat, &entry, &mut r);
        assert_eq!(r["code"]["coding"][0]["code"], json!("73211009"));
        assert_eq!(
            r["code"]["coding"][0]["system"],
            json!("http://snomed.info/sct")
        );
        assert_eq!(
            r["code"]["coding"][0]["display"],
            json!("Diabetes mellitus")
        );
    }

    // ── Full round trip: FHIR → build → reverse → equals the original mapped
    // fields. Uses the same BP template + mapping as the inbound end-to-end test.
    #[test]
    fn reverse_round_trip_equals_original_mapped_fields() {
        let wt = bp_web_template();
        let d = def(json!({
            "resource_type": "Observation",
            "template_id": "ehrbase_blood_pressure_simple.de.v0",
            "subject": { "reference_path": "subject.reference", "namespace": "fhir", "strip_prefix": "Patient/" },
            "context": {
                "ctx/language": "en", "ctx/territory": "US",
                "ctx/composer_name": "fhir-connector", "ctx/time": "2026-02-03T04:05:06Z"
            },
            "entries": [
                { "openehr_path": "encounter_training_sample/blood_pressure_training_sample:0/systolic",
                  "fhir_path": "component[0].valueQuantity.value",
                  "transform": { "kind": "quantity", "unit_path": "component[0].valueQuantity.unit" } },
                { "openehr_path": "encounter_training_sample/blood_pressure_training_sample:0/diastolic",
                  "fhir_path": "component[1].valueQuantity.value",
                  "transform": { "kind": "quantity", "unit_path": "component[1].valueQuantity.unit" } }
            ]
        }));
        let original = json!({
            "resourceType": "Observation",
            "subject": { "reference": "Patient/p-1" },
            "component": [
                { "valueQuantity": { "value": 118, "unit": "mm[Hg]" } },
                { "valueQuantity": { "value": 76, "unit": "mm[Hg]" } }
            ]
        });

        // FHIR → inbound build → COMPOSITION. Fixed ctx/time (ITS-REST
        // simplified_formats master04 §Context) keeps the build deterministic.
        let flat = build_flat(&original, &d, &CodeTranslations::default()).expect("build_flat");
        let comp = composition_from_flat(&flat, &wt, "2024-01-01T00:00:00Z").expect("from_flat");

        // COMPOSITION → reverse → FHIR (subject id is the post-strip subject).
        let reversed = to_fhir("Observation", &comp, &wt, &d, Some("p-1")).expect("reverse maps");

        // Equal to the original on every mapped field (magnitude compared
        // numerically — the FLAT round trip may re-render 118 as 118.0).
        let num = |v: &Value, ptr: &str| v.pointer(ptr).and_then(Value::as_f64);
        assert_eq!(
            num(&reversed, "/component/0/valueQuantity/value"),
            Some(118.0)
        );
        assert_eq!(
            num(&reversed, "/component/1/valueQuantity/value"),
            Some(76.0)
        );
        assert_eq!(
            reversed.pointer("/component/0/valueQuantity/unit"),
            Some(&json!("mm[Hg]"))
        );
        assert_eq!(
            reversed.pointer("/component/1/valueQuantity/unit"),
            Some(&json!("mm[Hg]"))
        );
        assert_eq!(
            reversed.pointer("/subject/reference"),
            Some(&json!("Patient/p-1")),
            "subject reconstructed with strip_prefix re-applied"
        );
    }
}
