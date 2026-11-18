// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `LINK`, `FEEDER_AUDIT`, `FEEDER_AUDIT_DETAILS`, `ISM_TRANSITION`,
//! `INSTRUCTION_DETAILS`, and the `_`-prefixed optional RM-attribute families.
//!
//! Covers ITS-REST `simplified_formats/master05-rm_mapping.adoc` §§LINK,
//! FEEDER_AUDIT, FEEDER_AUDIT_DETAILS, ISM_TRANSITION, INSTRUCTION_DETAILS, plus
//! the `_`-rows of the LOCATABLE / ENTRY / ELEMENT / EVENT_CONTEXT / PARTY /
//! DV_ORDERED / DV_TEXT tables (master04 §"RM Attributes prefix"). The
//! value-internal families (`_normal_range`, `_mapping`, …) reuse the builders in
//! [`super::data_values`]; the party / participation / reference shapes reuse
//! [`super::parties`].

#![expect(
    clippy::disallowed_types,
    reason = "a datum of the simplified wire is a JSON scalar (ITS-REST Simplified Formats); \
              the builders read it into typed RM and the emitters write it (#1694, #6003)"
)]

use serde_json::Value;

use super::{Attr, data_values, parties};
use crate::flat::error::FlatError;
use crate::flat::sim::SimNode;
use crate::flat::typed::{self, Build, Obj, o, s};

// ── sim → RM ──────────────────────────────────────────────────────────────────

/// One `_`-segment family → `(rm_attribute, value)` (see [`super::build_rm_attr`]).
#[expect(
    clippy::too_many_lines,
    reason = "one dispatch over the `_`-attribute families; the length is the size of that family set, not logic"
)]
pub(super) fn build_rm_attr(
    seg: &str,
    occurrences: &[SimNode],
    host_base: &str,
    path: &str,
) -> Result<Option<(&'static str, Attr)>, FlatError> {
    let name = seg.trim_start_matches('_');
    let first = || occurrences.iter().find(|o| !o.is_empty());
    // A single-valued family takes its value whole from `|raw` (master04 §Raw
    // canonical JSON) or builds it from the master05 suffixes.
    let single = |types: &[&str], build: fn(&SimNode) -> Result<Obj, FlatError>| {
        first()
            .map(|o| {
                super::raw_value(o, &format!("{path}/{seg}"), types).unwrap_or_else(|| build(o))
            })
            .transpose()
    };
    let leaf = |rm_type: &str| -> Result<Option<Attr>, FlatError> {
        Ok(match first() {
            Some(o) => data_values::build_leaf(o, rm_type, None)?.map(Attr::object),
            None => None,
        })
    };
    let out = match name {
        "uid" => match first().and_then(SimNode::bare).and_then(Value::as_str) {
            Some(v) => Some(("uid", Attr::object(uid_value(v, &format!("{path}/{seg}"))?))),
            None => None,
        },
        "link" => {
            // Indexed by the raw occurrence position (holes included) so a
            // reported key matches the client's own `_link:i` numbering.
            let mut links = Vec::new();
            for (i, occurrence) in occurrences.iter().enumerate() {
                if occurrence.is_empty() {
                    continue;
                }
                let at = format!("{path}/{seg}:{i}");
                links.push(match super::raw_value(occurrence, &at, &["LINK"]) {
                    Some(raw) => raw?,
                    None => build_link(occurrence, path, i)?,
                });
            }
            Some(("links", Attr::Many(links)))
        }
        "feeder_audit" => single(&["FEEDER_AUDIT"], build_feeder_audit)?
            .map(|v| ("feeder_audit", Attr::object(v))),
        "null_flavour" => leaf("DV_CODED_TEXT")?.map(|v| ("null_flavour", v)),
        "null_reason" => leaf("DV_TEXT")?.map(|v| ("null_reason", v)),
        "guideline_id" | "work_flow_id" => {
            let attr = if name == "guideline_id" {
                "guideline_id"
            } else {
                "workflow_id"
            };
            first()
                .map(parties::build_object_ref)
                .transpose()?
                .flatten()
                .map(|v| (attr, Attr::object(v)))
        }
        "provider" => single(super::PARTY_PROXY_CLASSES, |o| {
            parties::build_party(o, "PERSON")
        })?
        .map(|v| ("provider", Attr::object(v))),
        "other_participation" => Some((
            "other_participations",
            Attr::Many(participations(occurrences, seg, path)?),
        )),
        "participation" => Some((
            "participations",
            Attr::Many(participations(occurrences, seg, path)?),
        )),
        "instruction_details" => match first() {
            Some(o) => Some((
                "instruction_details",
                Attr::object(build_instruction_details(o, &format!("{path}/{seg}"))?),
            )),
            None => None,
        },
        "expiry_time" => leaf("DV_DATE_TIME")?.map(|v| ("expiry_time", v)),
        "wf_definition" => leaf("DV_PARSABLE")?.map(|v| ("wf_definition", v)),
        "identifier" => Some((
            "identifiers",
            Attr::Many(build_each(occurrences, parties::build_identifier)?),
        )),
        "reason" => Some((
            "reason",
            Attr::Many(build_each(occurrences, |o| match data_values::build_leaf(
                o, "DV_TEXT", None,
            )? {
                Some(text) => Ok(text),
                None => typed::dv_text(""),
            })?),
        )),
        "end_time" => leaf("DV_DATE_TIME")?.map(|v| ("end_time", v)),
        // EVENT_CONTEXT.location is a plain String (master05 §EVENT_CONTEXT note).
        "location" => first()
            .and_then(SimNode::bare)
            .map(|v| ("location", Attr::Datum(v.clone()))),
        "health_care_facility" => single(&["PARTY_IDENTIFIED"], |o| {
            parties::build_party(o, "ORGANISATION")
        })?
        .map(|v| ("health_care_facility", Attr::object(v))),
        // value-internal families (`T` = the host leaf type).
        "normal_range" => first()
            .map(|o| data_values::build_interval(o, host_base))
            .transpose()?
            .map(|v| ("normal_range", Attr::object(v))),
        "other_reference_ranges" => Some((
            "other_reference_ranges",
            Attr::Many(build_each(occurrences, |o| {
                data_values::build_reference_range(o, host_base)
            })?),
        )),
        "accuracy" => match first().and_then(SimNode::bare) {
            Some(v) => Some(("accuracy", Attr::object(data_values::duration(v)?))),
            None => None,
        },
        "thumbnail" => leaf("DV_MULTIMEDIA")?.map(|v| ("thumbnail", v)),
        "charset" => code_phrase_family(first())?.map(|v| ("charset", v)),
        "language" => code_phrase_family(first())?.map(|v| ("language", v)),
        "encoding" => code_phrase_family(first())?.map(|v| ("encoding", v)),
        "mapping" => Some((
            "mappings",
            Attr::Many(build_each(occurrences, data_values::build_term_mapping)?),
        )),
        _ => {
            return Err(FlatError::UnknownSuffix {
                rm_type: host_base.to_owned(),
                suffix: seg.to_owned(),
                path: path.to_owned(),
            });
        }
    };
    Ok(out)
}

/// The CODE_PHRASE a single-valued `_`-family occurrence builds.
///
/// # Errors
/// [`FlatError::Rm`] for a datum the CODE_PHRASE refuses.
fn code_phrase_family(occurrence: Option<&SimNode>) -> Result<Option<Attr>, FlatError> {
    Ok(match occurrence {
        Some(o) => data_values::build_code_phrase(o)?.map(Attr::object),
        None => None,
    })
}

/// The `PARTICIPATION` of each non-empty occurrence, in order, each taken
/// whole from `|raw` (master04 §Raw canonical JSON) or built from the
/// master05 §PARTICIPATION suffixes.
///
/// # Errors
/// [`FlatError::InvalidRaw`] for a `|raw` value that is no lone
/// `PARTICIPATION`.
fn participations(occurrences: &[SimNode], seg: &str, path: &str) -> Result<Vec<Obj>, FlatError> {
    occurrences
        .iter()
        .enumerate()
        .filter(|(_, o)| !o.is_empty())
        .map(|(i, o)| {
            super::raw_value(o, &format!("{path}/{seg}:{i}"), &["PARTICIPATION"])
                .unwrap_or_else(|| parties::build_participation(o))
        })
        .collect()
}

/// Apply `f` to each non-empty occurrence, in order.
fn build_each<F>(occurrences: &[SimNode], f: F) -> Result<Vec<Obj>, FlatError>
where
    F: Fn(&SimNode) -> Result<Obj, FlatError>,
{
    occurrences
        .iter()
        .filter(|o| !o.is_empty())
        .map(f)
        .collect()
}

/// A `::`-bearing id is an OBJECT_VERSION_ID; otherwise a HIER_OBJECT_ID (RM
/// support identifiers — both are valid `LOCATABLE.uid` UID_BASED_IDs).
fn uid_value(v: &str, path: &str) -> Result<Obj, FlatError> {
    if v.contains("::") {
        typed::object_version_id(v, path)
    } else {
        typed::hier_object_id(v, path)
    }
}

/// LINK (master05 §LINK): `type`/`meaning` are DV_TEXT, `target` is DV_EHR_URI.
///
/// All three suffixes are `Required: yes` in the master05 §LINK mapping table,
/// mirroring the RM's own 1..1 multiplicities
/// (`RM/docs/UML/classes/org.openehr.rm.common.link.adoc` §Attributes), so an
/// omitted one is reported as [`FlatError::MissingRequiredSuffix`] naming the
/// exact key rather than defaulted to an empty value: a fabricated empty
/// `meaning` would satisfy the RM's cardinality with data the client never
/// sent.
///
/// `index` is the `:i` occurrence index of the `_link` family, so the reported
/// key is the one the client can find in its own document.
fn build_link(node: &SimNode, path: &str, index: usize) -> Result<Obj, FlatError> {
    let required = |suffix: &str| -> Result<&str, FlatError> {
        node.attrs
            .get(suffix)
            .and_then(Value::as_str)
            .ok_or_else(|| FlatError::MissingRequiredSuffix {
                key: format!("{path}/_link:{index}|{suffix}"),
            })
    };
    let text = |value: &str| {
        openehr_rm::v1_2::data_types::text::dv_text::DvText::DvText(
            openehr_rm::v1_2::data_types::text::dv_text::DvTextData {
                value: value.to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: None,
                language: None,
                encoding: None,
            },
        )
    };
    let link = openehr_rm::v1_2::common::archetyped::link::Link {
        meaning: text(required("meaning")?),
        r#type: text(required("type")?),
        target: openehr_rm::v1_2::data_types::uri::dv_ehr_uri::DvEhrUri {
            value: required("target")?.to_owned(),
        },
    };
    Ok(Box::new(link))
}

/// FEEDER_AUDIT (master05 §FEEDER_AUDIT).
fn build_feeder_audit(node: &SimNode) -> Result<Obj, FlatError> {
    let mut fa = typed::new("FEEDER_AUDIT")?;
    if let Some(o) = single(node, "originating_system_audit") {
        fa.put(
            "originating_system_audit",
            typed::o(build_audit_details(o)?),
        )?;
    }
    if let Some(o) = single(node, "feeder_system_audit") {
        fa.put("feeder_system_audit", typed::o(build_audit_details(o)?))?;
    }
    // master05: only one of `original_content` / `original_content_multimedia`.
    if let Some(o) = single(node, "original_content")
        && let Some(dv) = data_values::build_leaf(o, "DV_PARSABLE", None)?
    {
        fa.put("original_content", typed::o(dv))?;
    }
    if let Some(o) = single(node, "original_content_multimedia")
        && let Some(dv) = data_values::build_leaf(o, "DV_MULTIMEDIA", None)?
    {
        fa.put("original_content", typed::o(dv))?;
    }
    for (seg, key) in [
        ("originating_system_item_id", "originating_system_item_ids"),
        ("feeder_system_item_id", "feeder_system_item_ids"),
    ] {
        if let Some(child) = node.children.get(seg) {
            for id in build_each(&child.occurrences, parties::build_identifier)? {
                fa.push_obj(key, id)?;
            }
        }
    }
    Ok(fa)
}

fn build_audit_details(node: &SimNode) -> Result<Obj, FlatError> {
    let system_id = node
        .attrs
        .get("system_id")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let mut d = typed::make("FEEDER_AUDIT_DETAILS", [("system_id", s(system_id))])?;
    if let Some(v) = node.attrs.get("version_id") {
        d.put_datum("version_id", v)?;
    }
    if let Some(v) = node.attrs.get("time") {
        let mut time = typed::new("DV_DATE_TIME")?;
        time.put_datum("value", v)?;
        d.put("time", o(time))?;
    }
    for party in ["location", "subject", "provider"] {
        if let Some(p) = single(node, party) {
            // master05 §FEEDER_AUDIT_DETAILS: `subject` is a PARTY_PROXY (may be
            // PARTY_SELF); `location`/`provider` are PARTY_IDENTIFIED.
            d.put(party, o(parties::build_party(p, "PERSON")?))?;
        }
    }
    Ok(d)
}

/// INSTRUCTION_DETAILS (master05 §INSTRUCTION_DETAILS): `|composition_uid` +
/// `|path` rebuild the mandatory `instruction_id` LOCATABLE_REF,
/// `|activity_id` the sibling String attribute.
///
/// `LOCATABLE_REF` inherits OBJECT_REF's mandatory `namespace` and `type`
/// (BASE base_types `LOCATABLE_REF`/`OBJECT_REF` classes: `namespace` 1..1,
/// `type` 1..1), but master05 defines **no** flat suffix for either — they are
/// derived here, not carried on the wire:
///
/// - `namespace` is `EHR` (the local EHR system context; OBJECT_REF's legal
///   values are `local`, `unknown`, or any `[a-zA-Z][a-zA-Z0-9_.:/&?=+-]*`
///   string — the same default `parties::build_object_ref` uses);
/// - `id` is an `OBJECT_VERSION_ID` when the uid carries the
///   `object_id::creating_system_id::version_tree_id` form (BASE
///   `master09-identification`), else a `HIER_OBJECT_ID`;
/// - `type` follows from that: a version uid names one `COMPOSITION`, a bare
///   object uid names the `VERSIONED_COMPOSITION`.
fn build_instruction_details(node: &SimNode, path: &str) -> Result<Obj, FlatError> {
    let uid = node
        .attrs
        .get("composition_uid")
        .and_then(Value::as_str)
        .unwrap_or("");
    let (ref_type, id) = if uid.contains("::") {
        ("COMPOSITION", typed::object_version_id(uid, path)?)
    } else {
        ("VERSIONED_COMPOSITION", typed::hier_object_id(uid, path)?)
    };
    let mut lref = typed::make(
        "LOCATABLE_REF",
        [
            ("namespace", s("EHR")),
            ("type", s(ref_type)),
            ("id", o(id)),
        ],
    )?;
    if let Some(p) = node.attrs.get("path") {
        lref.put_datum("path", p)?;
    }
    let mut details = typed::make("INSTRUCTION_DETAILS", [("instruction_id", o(lref))])?;
    if let Some(aid) = node.attrs.get("activity_id") {
        details.put_datum("activity_id", aid)?;
    }
    Ok(details)
}

fn single<'a>(node: &'a SimNode, name: &str) -> Option<&'a SimNode> {
    node.children.get(name).and_then(|c| c.occurrences.first())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flat::instance::{Instance, at};
    use crate::flat::map::emit::emit_rm_attrs;
    use openehr_rm::v1_2::access::RmValue;
    use serde_json::json;

    // master05 §LINK + §COMPOSITION `_uid`: round-trip through the RM-attr layer.
    #[test]
    fn uid_and_link_roundtrip() {
        let rm = json!({
            "_type": "OBSERVATION",
            "uid": {"_type": "HIER_OBJECT_ID", "value": "9fcc1c70-9349-444d-b9de-b1f3ddcc3c4e"},
            "links": [{"_type": "LINK",
                "type": {"_type": "DV_TEXT", "value": "problem"},
                "meaning": {"_type": "DV_TEXT", "value": "related"},
                "target": {"_type": "DV_EHR_URI", "value": "ehr://x"}}]
        });
        let mut out = SimNode::default();
        emit_rm_attrs(&rm, "OBSERVATION", &mut out);
        assert_eq!(
            out.child("_uid").and_then(SimNode::bare),
            Some(&json!("9fcc1c70-9349-444d-b9de-b1f3ddcc3c4e"))
        );
        let link = out.child("_link").unwrap();
        assert_eq!(link.attrs.get("type"), Some(&json!("problem")));
        assert_eq!(link.attrs.get("target"), Some(&json!("ehr://x")));

        let uid_occ = out.children["_uid"].occurrences.clone();
        let (attr, v) = build_rm_attr("_uid", &uid_occ, "OBSERVATION", "p")
            .unwrap()
            .unwrap();
        let v = v.canonical();
        assert_eq!(attr, "uid");
        assert_eq!(v["_type"], json!("HIER_OBJECT_ID"));
        let link_occ = out.children["_link"].occurrences.clone();
        let (attr, v) = build_rm_attr("_link", &link_occ, "OBSERVATION", "p")
            .unwrap()
            .unwrap();
        let v = v.canonical();
        assert_eq!(attr, "links");
        assert_eq!(v[0]["type"]["value"], json!("problem"));
        assert_eq!(v[0]["_type"], json!("LINK"));
    }

    /// A `_link:i` occurrence carrying exactly the listed suffixes.
    fn link_occurrence(suffixes: &[(&str, &str)]) -> SimNode {
        let mut node = SimNode::default();
        for (suffix, value) in suffixes {
            node.attrs.insert((*suffix).to_owned(), json!(*value));
        }
        node
    }

    // master05 §LINK marks `|type`, `|meaning` and `|target` all
    // `Required: yes`, mirroring the RM's 1..1 multiplicities
    // (`org.openehr.rm.common.link.adoc` §Attributes). A complete `_link:0`
    // builds; each partial input names the one key the client omitted rather
    // than silently defaulting it to an empty DV_TEXT/DV_EHR_URI.
    #[test]
    fn link_requires_all_three_suffixes() {
        let complete = link_occurrence(&[
            ("type", "problem"),
            ("meaning", "related"),
            ("target", "ehr://x"),
        ]);
        let (attr, v) = build_rm_attr(
            "_link",
            std::slice::from_ref(&complete),
            "OBSERVATION",
            "tpl/obs",
        )
        .expect("a complete LINK builds")
        .expect("the `_link` family yields a value");
        let v = v.canonical();
        assert_eq!(attr, "links");
        assert_eq!(v[0]["meaning"]["value"], json!("related"));
        assert_eq!(v[0]["target"]["_type"], json!("DV_EHR_URI"));

        for (missing, present) in [
            ("meaning", vec![("type", "problem"), ("target", "ehr://x")]),
            ("type", vec![("meaning", "related"), ("target", "ehr://x")]),
            ("target", vec![("type", "problem"), ("meaning", "related")]),
        ] {
            let partial = link_occurrence(&present);
            let err = build_rm_attr(
                "_link",
                std::slice::from_ref(&partial),
                "OBSERVATION",
                "tpl/obs",
            )
            .expect_err("a LINK missing a mandatory suffix must be refused");
            assert_eq!(
                err.to_string(),
                format!("tpl/obs/_link:0|{missing} is required")
            );
        }
    }

    // The reported key carries the occurrence's own `:i` index, so a client
    // reading the message finds the offending key in its own document.
    #[test]
    fn link_diagnostic_names_the_offending_occurrence() {
        let occurrences = [
            link_occurrence(&[
                ("type", "problem"),
                ("meaning", "related"),
                ("target", "ehr://x"),
            ]),
            link_occurrence(&[("type", "problem"), ("meaning", "related")]),
        ];
        let err = build_rm_attr("_link", &occurrences, "OBSERVATION", "tpl/obs")
            .expect_err("the second LINK is incomplete");
        assert_eq!(err.to_string(), "tpl/obs/_link:1|target is required");
    }

    // master05 §ELEMENT: `_null_flavour` (DV_CODED_TEXT) + `_null_reason` (DV_TEXT).
    #[test]
    fn null_flavour_and_reason() {
        let mut nf = SimNode::default();
        nf.attrs.insert("code".to_owned(), json!("253"));
        nf.attrs.insert("value".to_owned(), json!("unknown"));
        nf.attrs.insert("terminology".to_owned(), json!("openehr"));
        let (attr, v) = build_rm_attr("_null_flavour", std::slice::from_ref(&nf), "", "p")
            .unwrap()
            .unwrap();
        let v = v.canonical();
        assert_eq!(attr, "null_flavour");
        assert_eq!(v["defining_code"]["code_string"], json!("253"));
    }

    // master05 §ISM_TRANSITION `/_reason:i` (DV_TEXT list).
    #[test]
    fn ism_reason_list() {
        let mut r0 = SimNode::default();
        r0.attrs.insert(String::new(), json!("reason 1"));
        let (attr, v) = build_rm_attr("_reason", std::slice::from_ref(&r0), "ISM_TRANSITION", "p")
            .unwrap()
            .unwrap();
        let v = v.canonical();
        assert_eq!(attr, "reason");
        assert_eq!(v[0]["value"], json!("reason 1"));
        assert_eq!(v[0]["_type"], json!("DV_TEXT"));
    }

    // master05 §FEEDER_AUDIT: originating_system_audit round-trip.
    #[test]
    fn feeder_audit_roundtrip() {
        let rm = json!({
            "_type": "OBSERVATION",
            "feeder_audit": {"_type": "FEEDER_AUDIT",
                "originating_system_audit": {"_type": "FEEDER_AUDIT_DETAILS",
                    "system_id": "orig", "version_id": "final"}}
        });
        let mut out = SimNode::default();
        emit_rm_attrs(&rm, "OBSERVATION", &mut out);
        let fa = out.child("_feeder_audit").unwrap();
        let osa = fa.child("originating_system_audit").unwrap();
        assert_eq!(osa.attrs.get("system_id"), Some(&json!("orig")));
        assert_eq!(osa.attrs.get("version_id"), Some(&json!("final")));

        let occ = out.children["_feeder_audit"].occurrences.clone();
        let (attr, v) = build_rm_attr("_feeder_audit", &occ, "OBSERVATION", "p")
            .unwrap()
            .unwrap();
        let v = v.canonical();
        assert_eq!(attr, "feeder_audit");
        assert_eq!(v["originating_system_audit"]["system_id"], json!("orig"));
        assert_eq!(v["originating_system_audit"]["version_id"], json!("final"));
    }

    // An unrecognised `_`-segment is rejected (master05 per-type tables).
    #[test]
    fn unknown_rm_attr_rejected() {
        let node = SimNode::default();
        assert!(matches!(
            build_rm_attr(
                "_frobnicate",
                std::slice::from_ref(&node),
                "OBSERVATION",
                "p"
            ),
            Err(FlatError::UnknownSuffix { .. })
        ));
    }

    // master05 §INSTRUCTION_DETAILS: the three flat suffixes sit on the
    // `_instruction_details` node itself (`|path`, `|composition_uid`,
    // `|activity_id`) — there is no nested `instruction_id` node and no
    // OBJECT_REF suffix.
    #[test]
    fn instruction_details_roundtrip() {
        let rm = json!({
            "_type": "ACTION",
            "instruction_details": {"_type": "INSTRUCTION_DETAILS",
                "instruction_id": {"_type": "LOCATABLE_REF", "namespace": "EHR",
                    "type": "COMPOSITION",
                    "id": {"_type": "OBJECT_VERSION_ID", "value": "4cdc3017-cf42-4f2f-9c5e-5f39e8e64a7e::example.org::1"},
                    "path": "/content[x]"},
                "activity_id": "activities[at0001]"}
        });
        let mut out = SimNode::default();
        emit_rm_attrs(&rm, "ACTION", &mut out);
        let det = out.child("_instruction_details").unwrap();
        assert_eq!(det.attrs.get("path"), Some(&json!("/content[x]")));
        assert_eq!(
            det.attrs.get("composition_uid"),
            Some(&json!(
                "4cdc3017-cf42-4f2f-9c5e-5f39e8e64a7e::example.org::1"
            ))
        );
        assert_eq!(
            det.attrs.get("activity_id"),
            Some(&json!("activities[at0001]"))
        );
        assert!(det.children.is_empty(), "no nested instruction_id node");

        let occ = out.children["_instruction_details"].occurrences.clone();
        let (attr, v) = build_rm_attr("_instruction_details", &occ, "ACTION", "p")
            .unwrap()
            .unwrap();
        let v = v.canonical();
        assert_eq!(attr, "instruction_details");
        assert_eq!(v["activity_id"], json!("activities[at0001]"));
        assert_eq!(v["instruction_id"]["path"], json!("/content[x]"));
        assert_eq!(
            v["instruction_id"]["id"]["value"],
            json!("4cdc3017-cf42-4f2f-9c5e-5f39e8e64a7e::example.org::1")
        );
        assert_eq!(
            v["instruction_id"]["id"]["_type"],
            json!("OBJECT_VERSION_ID")
        );
        assert_eq!(v["instruction_id"]["type"], json!("COMPOSITION"));

        // A bare (non-versioned) composition uid names the versioned object.
        let mut bare = SimNode::default();
        bare.attrs.insert(
            "composition_uid".to_owned(),
            json!("4cdc3017-cf42-4f2f-9c5e-5f39e8e64a7e"),
        );
        let (_, v) = build_rm_attr("_instruction_details", &[bare], "ACTION", "p")
            .unwrap()
            .unwrap();
        // The bare node carries no `|activity_id`, which INSTRUCTION_DETAILS
        // makes mandatory, so the partial object is read where it stands.
        let Attr::One(RmValue::Object(details)) = v else {
            panic!("an INSTRUCTION_DETAILS object, got {v:?}")
        };
        let details = typed::view(&*details);
        assert_eq!(
            at(details, &["instruction_id", "id"]).and_then(Instance::rm_type),
            Some("HIER_OBJECT_ID")
        );
        assert_eq!(
            at(details, &["instruction_id", "type"]).and_then(Instance::as_str),
            Some("VERSIONED_COMPOSITION")
        );
    }
}
