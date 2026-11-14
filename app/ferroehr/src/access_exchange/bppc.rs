// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access directives as IHE BPPC Patient Privacy Acknowledgement Documents
//! (ITI TF-1 chapter 19, ITI TF-3 section 5.1, under
//! `docs/specs/ihe-consents/profiles.ihe.net/ITI/TF/`).
//!
//! BPPC records that a patient acknowledged a privacy policy the policy
//! domain publishes, and carries no structured rule: "the content of the
//! policy is human readable text" (TF-1 19.1.1). Each `serviceEvent` names
//! one policy, its `code` the Patient Privacy Policy Identifier and its
//! `codeSystem` the policy domain's (TF-3 5.1.2.2.6), and its
//! `effectiveTime` the range the acknowledgement holds, `low` required and
//! the same in every service event of one document (5.1.2.2.7).
//!
//! An acknowledgement maps to the `ACCESS_POLICY.catalogue` entry with that
//! identifier, copied into the patient's `EHR_ACCESS` with `effectiveTime` as
//! its `time_validity` and the composition holding the document as
//! `evidence`; a policy the catalogue does not hold is refused, since
//! FerroEHR cannot know what it means. Export writes one document per
//! distinct `time_validity` of the directives copied from the catalogue
//! ("If different effectiveTimes are needed then different documents should
//! be generated", 5.1.2.2.7), and names every other directive as one an
//! acknowledgement cannot state. Storing the document as a composition is
//! the caller's, under the deployment's own consent template, since
//! FerroEHR reads no archetype id: our own design.

use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::v1_2::data_types::uri::dv_ehr_uri::DvEhrUri;

use crate::access_exchange::rm::{bounds, interval, list, members, text_of};
use crate::access_exchange::xml::{Node, parse};
use crate::access_exchange::{Format, Loss, Refused, catalogue_entry};
use crate::versioning::audit::dv_date_time_text;

/// The template of a Patient Privacy Acknowledgement Document (TF-3 5.1.2).
pub const DOCUMENT_TEMPLATE: &str = "1.3.6.1.4.1.19376.1.5.3.1.1.7";
/// The template of one with a scanned part (TF-3 5.1.3).
pub const SCANNED_TEMPLATE: &str = "1.3.6.1.4.1.19376.1.5.3.1.1.7.1";
/// The template of the Medical Document content module it inherits from
/// (TF-3 5.1.2.2).
pub const MEDICAL_DOCUMENT_TEMPLATE: &str = "1.3.6.1.4.1.19376.1.5.3.1.1.1";
/// The template of a Patient Privacy Acknowledgement Service Event (TF-3
/// 5.1.2.2.1).
pub const SERVICE_EVENT_TEMPLATE: &str = "1.3.6.1.4.1.19376.1.5.3.1.2.6";

/// The element path of a service event.
const SERVICE_EVENT: &str = "ClinicalDocument/documentationOf/serviceEvent";

/// What an import reads beside the document.
#[derive(Debug, Clone, Copy)]
pub struct ImportContext<'a> {
    /// The `ACCESS_POLICY.catalogue` in force.
    pub catalogue: &'a [AccessDirective],
    /// The `ehr:` URI of the composition that holds the document.
    pub evidence: &'a str,
}

/// What an export reads beside the directives.
#[derive(Debug, Clone, Copy)]
pub struct ExportContext<'a> {
    /// The `ACCESS_POLICY.catalogue` in force.
    pub catalogue: &'a [AccessDirective],
    /// The patient's identifier as a CDA `II` root, when the EHR subject
    /// names one.
    pub patient: Option<&'a str>,
    /// The controller's identifier as a CDA `II` root, the author and
    /// custodian, when the policy names one.
    pub controller: Option<&'a str>,
    /// The moment of the export, an ISO 8601 date and time.
    pub now: &'a str,
}

/// The `low` and `high` of an `effectiveTime`, as HL7 `TS` values.
type Range = (String, Option<String>);

/// What an export writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Exported {
    /// One document per distinct `time_validity`.
    pub documents: Vec<String>,
    /// Each directive no acknowledgement states, by its position, with why.
    pub refused: Vec<(usize, Refused)>,
}

fn malformed(element: impl Into<String>, why: &str) -> Refused {
    Refused::new(Format::Bppc, element, Loss::Malformed(why.to_owned()))
}

/// Reads `text`, a BPPC document, as the catalogue entries it acknowledges.
///
/// # Errors
/// [`Refused`] for a document that is no BPPC acknowledgement, a service
/// event that breaks TF-3 5.1.2.2, or a policy the catalogue does not hold.
pub fn import(text: &str, context: &ImportContext<'_>) -> Result<Vec<AccessDirective>, Refused> {
    let root = parse(text).map_err(|error| {
        Refused::new(
            Format::Bppc,
            "ClinicalDocument",
            Loss::Malformed(error.to_string()),
        )
    })?;
    if root.name != "ClinicalDocument" {
        return Err(Refused::new(
            Format::Bppc,
            root.name.clone(),
            Loss::NoDirectiveForm,
        ));
    }
    let templates: Vec<&str> = root
        .all("templateId")
        .filter_map(|t| t.get("root"))
        .collect();
    if !templates.contains(&DOCUMENT_TEMPLATE) && !templates.contains(&SCANNED_TEMPLATE) {
        return Err(Refused::new(
            Format::Bppc,
            "ClinicalDocument/templateId",
            Loss::Code(templates.join(", ")),
        ));
    }
    let events: Vec<&Node> = root
        .all("documentationOf")
        .flat_map(|d| d.all("serviceEvent"))
        .filter(|e| {
            e.all("templateId")
                .any(|t| t.get("root") == Some(SERVICE_EVENT_TEMPLATE))
        })
        .collect();
    if events.is_empty() {
        return Err(malformed(
            SERVICE_EVENT,
            "a document acknowledges at least one policy (TF-3 5.1.2.2)",
        ));
    }
    let instructed = root
        .first("effectiveTime")
        .and_then(|t| t.get("value"))
        .map(|ts| iso(ts, "ClinicalDocument/effectiveTime"))
        .transpose()?;
    let mut range: Option<(&str, Option<&str>)> = None;
    let mut out = Vec::new();
    for event in events {
        let code = event.first("code");
        let (Some(policy), Some(system)) = (
            code.and_then(|c| c.get("code")),
            code.and_then(|c| c.get("codeSystem")),
        ) else {
            return Err(malformed(
                format!("{SERVICE_EVENT}/code"),
                "a policy needs its code and codeSystem (TF-3 5.1.2.2.6)",
            ));
        };
        let time = event.first("effectiveTime");
        let high = time
            .and_then(|t| t.first("high"))
            .and_then(|h| h.get("value"));
        let Some(low) = time
            .and_then(|t| t.first("low"))
            .and_then(|l| l.get("value"))
        else {
            return Err(malformed(
                format!("{SERVICE_EVENT}/effectiveTime/low"),
                "the low value must be provided (TF-3 5.1.2.2.7)",
            ));
        };
        if range.is_some_and(|seen| seen != (low, high)) {
            return Err(malformed(
                format!("{SERVICE_EVENT}/effectiveTime"),
                "the effectiveTime must be the same for all service events (TF-3 5.1.2.2.7)",
            ));
        }
        range = Some((low, high));
        let entry = catalogue_entry(context.catalogue, Some(system), policy).ok_or_else(|| {
            Refused::new(
                Format::Bppc,
                format!("{SERVICE_EVENT}/code"),
                Loss::NotInCatalogue(format!("{system}|{policy}")),
            )
        })?;
        let mut copy = entry.clone();
        let element = format!("{SERVICE_EVENT}/effectiveTime");
        copy.time_validity = Some(interval(
            Some(iso(low, &element)?),
            high.map(|h| iso(h, &element)).transpose()?,
        ));
        copy.evidence = list(vec![DvEhrUri {
            value: context.evidence.to_owned(),
        }]);
        copy.instructed = instructed.clone().map(dv_date_time_text);
        out.push(copy);
    }
    Ok(out)
}

/// Writes the directives copied from the catalogue as BPPC documents.
#[must_use]
pub fn export(directives: &[AccessDirective], context: &ExportContext<'_>) -> Exported {
    let mut exported = Exported::default();
    let mut groups: Vec<(Range, Vec<&AccessDirective>)> = Vec::new();
    for (at, directive) in directives.iter().enumerate() {
        match acknowledged(directive, context.catalogue) {
            Ok(range) => match groups.iter_mut().find(|(seen, _)| *seen == range) {
                Some((_, group)) => group.push(directive),
                None => groups.push((range, vec![directive])),
            },
            Err(why) => exported.refused.push((at, why)),
        }
    }
    for ((low, high), group) in groups {
        exported
            .documents
            .push(document((&low, high.as_deref()), &group, context));
    }
    exported
}

/// The `effectiveTime` of `directive` as HL7 `TS` values, when it is a copy
/// of a catalogue entry an acknowledgement states whole.
fn acknowledged(
    directive: &AccessDirective,
    catalogue: &[AccessDirective],
) -> Result<Range, Refused> {
    let not_stated = |attribute: &str| {
        Refused::new(
            Format::Directive,
            attribute,
            Loss::NoFormatForm(Format::Bppc),
        )
    };
    let entry = members(directive.identifiers.as_ref())
        .iter()
        .find_map(|i| catalogue_entry(catalogue, i.issuer.as_deref(), &i.id))
        .ok_or_else(|| not_stated("identifiers"))?;
    let same = entry.kind == directive.kind
        && entry.effect == directive.effect
        && entry.applies_to == directive.applies_to
        && entry.operations == directive.operations
        && entry.purposes == directive.purposes
        && entry.data == directive.data
        && entry.emergency_override == directive.emergency_override
        && entry.exceptions == directive.exceptions;
    if !same {
        return Err(not_stated("applies_to"));
    }
    let (low, high) = directive
        .time_validity
        .as_ref()
        .and_then(bounds)
        .ok_or_else(|| not_stated("time_validity"))?;
    let low = low.ok_or_else(|| not_stated("time_validity"))?;
    let ts_of = |value: &str| ts(value).ok_or_else(|| not_stated("time_validity"));
    Ok((ts_of(&low)?, high.as_deref().map(ts_of).transpose()?))
}

/// The CDA `II` of `root`, or an unknown one.
fn id(root: Option<&str>) -> Node {
    match root {
        Some(root) => Node::new("id").attr("root", root),
        None => Node::new("id").attr("nullFlavor", "UNK"),
    }
}

/// The header of an acknowledgement document (TF-3 5.1.2.2, the sample of
/// Figure 5.1.2.2-1), with the patient, author and custodian CDA R2 asks of
/// every document.
fn header(context: &ExportContext<'_>, now: &str) -> Node {
    Node::new("ClinicalDocument")
        .attr("xmlns", "urn:hl7-org:v3")
        .child(
            Node::new("typeId")
                .attr("root", "2.16.840.1.113883.1.3")
                .attr("extension", "POCD_HD000040"),
        )
        .child(Node::new("templateId").attr("root", MEDICAL_DOCUMENT_TEMPLATE))
        .child(Node::new("templateId").attr("root", DOCUMENT_TEMPLATE))
        .child(id(Some(&uuid::Uuid::now_v7().to_string())))
        .child(
            Node::new("code")
                .attr("code", "57016-8")
                .attr("displayName", "PATIENT PRIVACY ACKNOWLEDGEMENT")
                .attr("codeSystem", "2.16.840.1.113883.6.1")
                .attr("codeSystemName", "LOINC"),
        )
        .child(Node::new("title").with_text("Patient Privacy Acknowledgement"))
        .child(Node::new("effectiveTime").attr("value", now))
        .child(
            Node::new("confidentialityCode")
                .attr("code", "N")
                .attr("codeSystem", "2.16.840.1.113883.5.25"),
        )
        .child(Node::new("languageCode").attr("code", "en-US"))
        .child(Node::new("recordTarget").child(Node::new("patientRole").child(id(context.patient))))
        .child(
            Node::new("author")
                .child(Node::new("time").attr("value", now))
                .child(Node::new("assignedAuthor").child(id(context.controller))),
        )
        .child(
            Node::new("custodian").child(Node::new("assignedCustodian").child(
                Node::new("representedCustodianOrganization").child(id(context.controller)),
            )),
        )
}

/// One acknowledgement document of `group`, all holding over `range`.
fn document(
    (low, high): (&str, Option<&str>),
    group: &[&AccessDirective],
    context: &ExportContext<'_>,
) -> String {
    let now = ts(context.now).unwrap_or_default();
    let mut root = header(context, &now);
    let mut names = Vec::new();
    for directive in group {
        let Some(identifier) = members(directive.identifiers.as_ref())
            .iter()
            .find(|i| catalogue_entry(context.catalogue, i.issuer.as_deref(), &i.id).is_some())
        else {
            continue;
        };
        let name = text_of(&directive.name);
        let mut time = Node::new("effectiveTime").child(Node::new("low").attr("value", low));
        if let Some(high) = high {
            time = time.child(Node::new("high").attr("value", high));
        }
        let mut code = Node::new("code")
            .attr("code", &identifier.id)
            .attr("displayName", name);
        if let Some(system) = &identifier.issuer {
            code = code.attr("codeSystem", system);
        }
        root = root.child(
            Node::new("documentationOf").attr("typeCode", "DOC").child(
                Node::new("serviceEvent")
                    .attr("classCode", "ACT")
                    .attr("moodCode", "EVN")
                    .child(Node::new("templateId").attr("root", SERVICE_EVENT_TEMPLATE))
                    .child(id(Some(&uuid::Uuid::now_v7().to_string())))
                    .child(code)
                    .child(time),
            ),
        );
        names.push(name);
    }
    root.child(
        Node::new("component").child(
            Node::new("structuredBody").child(
                Node::new("component").child(
                    Node::new("section")
                        .child(Node::new("title").with_text("Privacy policies acknowledged"))
                        .child(Node::new("text").with_text(&names.join("; "))),
                ),
            ),
        ),
    )
    .document()
}

/// The digit counts an HL7 v3 `TS` takes before any fraction: a year, a
/// month, a day, an hour, a minute and a second.
const TS_LENGTHS: [usize; 6] = [4, 6, 8, 10, 12, 14];

/// An HL7 v3 `TS` (`YYYY[MM[DD[HH[MM[SS[.S+]]]]]][+|-ZZZZ]`) as ISO 8601.
fn iso(ts: &str, element: &str) -> Result<String, Refused> {
    let malformed = || {
        Refused::new(
            Format::Bppc,
            element,
            Loss::Malformed(format!("{ts} is no HL7 TS")),
        )
    };
    let (body, zone) = match ts.find(['+', '-']) {
        Some(at) => (ts.get(..at).unwrap_or_default(), ts.get(at..)),
        None => (ts, None),
    };
    let (digits, fraction) = match body.split_once('.') {
        Some((digits, fraction)) => (digits, Some(fraction)),
        None => (body, None),
    };
    if !digits.bytes().all(|b| b.is_ascii_digit()) || !TS_LENGTHS.contains(&digits.len()) {
        return Err(malformed());
    }
    let part = |at: usize| digits.get(at..at.saturating_add(2)).unwrap_or_default();
    let mut out = digits.get(..4).unwrap_or_default().to_owned();
    for (at, separator) in [(4, '-'), (6, '-'), (8, 'T'), (10, ':'), (12, ':')] {
        if digits.len() > at {
            out.push(separator);
            out.push_str(part(at));
        }
    }
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    match zone {
        Some("+0000" | "-0000") => out.push('Z'),
        Some(zone) if zone.len() == 5 => {
            out.push_str(zone.get(..3).unwrap_or_default());
            out.push(':');
            out.push_str(zone.get(3..).unwrap_or_default());
        }
        Some(_) => return Err(malformed()),
        None => {}
    }
    Ok(out)
}

/// An ISO 8601 date or date and time as an HL7 v3 `TS`; `None` for a form
/// `TS` has no digits for.
fn ts(iso: &str) -> Option<String> {
    let (body, zone) = if let Some(body) = iso.strip_suffix('Z') {
        (body, Some("+0000".to_owned()))
    } else {
        match iso.get(10..).and_then(|time| time.rfind(['+', '-'])) {
            Some(at) => {
                let at = at.saturating_add(10);
                (iso.get(..at)?, Some(iso.get(at..)?.replace(':', "")))
            }
            None => (iso, None),
        }
    };
    let (main, fraction) = match body.split_once('.') {
        Some((main, fraction)) => (main, Some(fraction)),
        None => (body, None),
    };
    let mut out: String = main
        .chars()
        .filter(|c| !matches!(c, '-' | ':' | 'T'))
        .collect();
    if !out.bytes().all(|b| b.is_ascii_digit()) || !TS_LENGTHS.contains(&out.len()) {
        return None;
    }
    if let Some(fraction) = fraction {
        out.push('.');
        out.push_str(fraction);
    }
    if let Some(zone) = zone {
        out.push_str(&zone);
    }
    Some(out)
}

#[cfg(test)]
mod tests;
