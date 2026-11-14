// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An XACML `Target` and `Condition` read into and written from the scope
//! and validity of a directive, by the attributes of APPC TF-3 5.6.2.1.4 to
//! 5.6.2.1.6.
//!
//! A `Target` holds `Subjects`, `Resources` and `Actions`, each a list of
//! which any one matches, each made of matches that must all hold (XACML
//! 5.5 to 5.17). One `Subject` is one actor scope, one `Resource` one data
//! scope, one `Action` one operation. A `Condition` is read only as the plain
//! interval over the current date and time FerroEHR writes for
//! `time_validity`; any other condition is refused.

use openehr_base::v1_3::base_types::identification::generic_id::GenericId;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    OperationClass, TERMINOLOGY_ID_ACCESS_CONTROL, Term,
};
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::quantity::dv_interval::DvInterval;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::access_exchange::rm::{bounds, interval, list, members, term};
use crate::access_exchange::xml::Node;
use crate::access_exchange::{Format, Loss, Refused};
use crate::access_log::entry::coded;

/// User ID, `xs:string` (APPC TF-3 5.6.2.1.4.1).
pub(super) const SUBJECT_ID: &str = "urn:oasis:names:tc:xacml:1.0:subject:subject-id";
/// User Organization ID, `xs:anyURI` (5.6.2.1.4.3).
pub(super) const ORGANIZATION_ID: &str = "urn:oasis:names:tc:xspa:1.0:subject:organization-id";
/// User Role, `CV` (5.6.2.1.4.6).
pub(super) const ROLE: &str = "urn:oasis:names:tc:xacml:2.0:subject:role";
/// Purpose Of Use, `CV` (5.6.2.1.4.7).
pub(super) const PURPOSE: &str = "urn:oasis:names:tc:xspa:1.0:subject:purposeofuse";
/// Patient ID, `II` (5.6.2.1.5.1.5).
pub(super) const PATIENT_ID: &str = "urn:ihe:iti:ser:2016:patient-id";
/// Confidentiality Code, `CV` (5.6.2.1.5.2.2).
pub(super) const CONFIDENTIALITY_CODE: &str = "urn:ihe:iti:appc:2016:confidentiality-code";
/// Class Code, `CV` (5.6.2.1.5.2.1).
pub(super) const CLASS_CODE: &str = "urn:ihe:iti:appc:2016:document-entry:class-code";
/// Type Code, `CV` (5.6.2.1.5.2.13).
pub(super) const TYPE_CODE: &str = "urn:ihe:iti:appc:2016:document-entry:type-code";
/// Source System ID, `xs:anyURI` (5.6.2.1.5.1.6).
pub(super) const SOURCE_SYSTEM_ID: &str = "urn:ihe:iti:appc:2016:source-system-id";
/// Author Institution ID (5.6.2.1.5.1.1).
pub(super) const AUTHOR_INSTITUTION_ID: &str = "urn:ihe:iti:appc:2016:author-institution:id";
/// The action, `xs:anyURI` (5.6.2.1.6.1).
pub(super) const ACTION_ID: &str = "urn:oasis:names:tc:xacml:1.0:action:action-id";
/// The current date and time (XACML 10.2.6).
const CURRENT_DATE_TIME: &str = "urn:oasis:names:tc:xacml:1.0:environment:current-dateTime";

const XS_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";
const XS_ANY_URI: &str = "http://www.w3.org/2001/XMLSchema#anyURI";
const XS_DATE_TIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const HL7_CV: &str = "urn:hl7-org:v3#CV";
const HL7_II: &str = "urn:hl7-org:v3#II";
const FUNCTION: &str = "urn:oasis:names:tc:xacml:1.0:function:";

/// The APPC action URIs (5.6.2.1.6.1) and the operation class each reads
/// as: our own design, a query response a query, a retrieve response a read
/// and a register or provide a write. The first of each class is the one an
/// export writes.
const ACTIONS: [(&str, OperationClass); 10] = [
    (
        "urn:ihe:iti:2007:RegistryStoredQueryResponse",
        OperationClass::Query,
    ),
    (
        "urn:ihe:iti:2007:CrossGatewayQueryResponse",
        OperationClass::Query,
    ),
    (
        "urn:ihe:iti:2009:MultiPatientStoredQueryResponse",
        OperationClass::Query,
    ),
    (
        "urn:ihe:iti:2007:RetrieveDocumentSetResponse",
        OperationClass::Read,
    ),
    (
        "urn:ihe:iti:2007:CrossGatewayRetrieveResponse",
        OperationClass::Read,
    ),
    (
        "urn:ihe:rad:2009:RetrieveImagingDocumentSetResponse",
        OperationClass::Read,
    ),
    (
        "urn:ihe:rad:2011:CrossGatewayRetrieveImagingDocumentSetResponse",
        OperationClass::Read,
    ),
    (
        "urn:ihe:iti:2007:ProvideAndRegisterDocumentSet-b",
        OperationClass::Write,
    ),
    (
        "urn:ihe:iti:2007:RegisterDocumentSet-b",
        OperationClass::Write,
    ),
    (
        "urn:ihe:iti:2010:RegisterOnDemandDocumentEntry",
        OperationClass::Write,
    ),
];

/// What one `Target` states.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Scope {
    /// One actor scope per `Subject` that names an actor.
    pub(super) actors: Vec<AccessActorScope>,
    /// The purposes the `Subject`s match.
    pub(super) purposes: Vec<DvCodedText>,
    /// One data scope per `Resource` that names data.
    pub(super) data: Vec<AccessDataScope>,
    /// One operation per `Action`.
    pub(super) operations: Vec<DvCodedText>,
    /// Whether a `Resource` matches the patient.
    pub(super) patient: bool,
}

impl Scope {
    /// Sets the scope on `target`, refusing an attribute both state.
    pub(super) fn apply(self, target: &mut AccessDirective, element: &str) -> Result<(), Refused> {
        let conflict = |what: &str| refused(format!("{element}/{what}"), Loss::NoDirectiveForm);
        if !self.actors.is_empty() {
            if target.applies_to.is_some() {
                return Err(conflict("Subjects"));
            }
            target.applies_to = list(self.actors);
        }
        if !self.purposes.is_empty() {
            if target.purposes.is_some() {
                return Err(conflict("Subjects"));
            }
            target.purposes = list(self.purposes);
        }
        if !self.data.is_empty() {
            if target.data.is_some() {
                return Err(conflict("Resources"));
            }
            target.data = list(self.data);
        }
        if !self.operations.is_empty() {
            if target.operations.is_some() {
                return Err(conflict("Actions"));
            }
            target.operations = list(self.operations);
        }
        Ok(())
    }
}

fn refused(element: impl Into<String>, loss: Loss) -> Refused {
    Refused::new(Format::Appc, element, loss)
}

/// One match of a target: its attribute id and value.
struct Match<'a> {
    attribute: &'a str,
    value: &'a Node,
}

/// The matches of one `Subject`, `Resource` or `Action` named `kind`.
fn matches<'a>(group: &'a Node, kind: &str, element: &str) -> Result<Vec<Match<'a>>, Refused> {
    let match_name = format!("{kind}Match");
    let designator = format!("{kind}AttributeDesignator");
    let mut out = Vec::new();
    for child in &group.children {
        if child.name != match_name {
            return Err(refused(
                format!("{element}/{}", child.name),
                Loss::NoDirectiveForm,
            ));
        }
        let (Some(value), Some(attribute)) = (
            child.first("AttributeValue"),
            child.first(&designator).and_then(|d| d.get("AttributeId")),
        ) else {
            return Err(refused(
                format!("{element}/{match_name}"),
                Loss::Malformed(format!("a match names its value and its {designator}")),
            ));
        };
        out.push(Match { attribute, value });
    }
    Ok(out)
}

/// A `CV` attribute value as a `DV_CODED_TEXT`, its `codeSystem` the
/// terminology (APPC TF-3 5.6.2.1.2).
fn cv(value: &Node, element: &str) -> Result<DvCodedText, Refused> {
    let node = value.first("CodedValue");
    match (
        node.and_then(|n| n.get("code")),
        node.and_then(|n| n.get("codeSystem")),
    ) {
        (Some(code), Some(system)) => Ok(coded(
            system,
            code,
            node.and_then(|n| n.get("displayName")).unwrap_or(code),
        )),
        _ => Err(refused(
            element,
            Loss::Malformed("a CV has a CodedValue with code and codeSystem".to_owned()),
        )),
    }
}

/// The party a subject attribute names: a `HIER_OBJECT_ID` when the value is
/// one (bare, or a `urn:uuid:`), else a `GENERIC_ID` whose scheme is the
/// attribute id.
fn party(value: &str, attribute: &str, rm_type: &str) -> PartyRef {
    let bare = value.strip_prefix("urn:uuid:").unwrap_or(value);
    // NOTE: BASE `HIER_OBJECT_ID`, a value outside its form is legitimately an id of
    // another scheme.
    let id = HierObjectId::new(bare).map_or_else(
        |_| {
            ObjectId::GenericId(GenericId {
                value: value.to_owned(),
                scheme: attribute.to_owned(),
            })
        },
        ObjectId::HierObjectId,
    );
    PartyRef {
        namespace: "demographic".to_owned(),
        r#type: rm_type.to_owned(),
        id,
    }
}

/// Reads `target`, named `element`.
pub(super) fn read(target: Option<&Node>, element: &str) -> Result<Scope, Refused> {
    let mut scope = Scope::default();
    let Some(target) = target else {
        return Ok(scope);
    };
    let subjects: Vec<&Node> = target
        .first("Subjects")
        .map(|s| s.all("Subject").collect())
        .unwrap_or_default();
    for subject in &subjects {
        let at = format!("{element}/Subjects/Subject");
        let mut actor = AccessActorScope {
            party: None,
            role: None,
            relationship: None,
            caller_kind: None,
        };
        for Match { attribute, value } in matches(subject, "Subject", &at)? {
            let at = format!("{at}/SubjectMatch");
            match attribute {
                SUBJECT_ID => actor.party = Some(party(value.text.trim(), SUBJECT_ID, "PERSON")),
                ORGANIZATION_ID => {
                    actor.party = Some(party(value.text.trim(), ORGANIZATION_ID, "ORGANISATION"));
                }
                ROLE => actor.role = Some(cv(value, &at)?),
                PURPOSE if subjects.len() == 1 => scope.purposes.push(cv(value, &at)?),
                other => return Err(refused(at, Loss::Code(other.to_owned()))),
            }
        }
        if actor.party.is_some() || actor.role.is_some() {
            scope.actors.push(actor);
        }
    }
    if let Some(resources) = target.first("Resources") {
        for resource in resources.all("Resource") {
            let at = format!("{element}/Resources/Resource");
            let mut data = blank_data();
            let mut named = false;
            for Match { attribute, value } in matches(resource, "Resource", &at)? {
                let at = format!("{at}/ResourceMatch");
                named |= attribute != PATIENT_ID;
                match attribute {
                    PATIENT_ID => scope.patient = true,
                    CONFIDENTIALITY_CODE => push(&mut data.sensitivities, cv(value, &at)?),
                    CLASS_CODE | TYPE_CODE => push(&mut data.categories, cv(value, &at)?),
                    SOURCE_SYSTEM_ID | AUTHOR_INSTITUTION_ID => {
                        push(&mut data.origins, value.text.trim().to_owned());
                    }
                    other => return Err(refused(at, Loss::Code(other.to_owned()))),
                }
            }
            if named {
                scope.data.push(data);
            }
        }
    }
    if let Some(actions) = target.first("Actions") {
        for action in actions.all("Action") {
            let at = format!("{element}/Actions/Action");
            for Match { attribute, value } in matches(action, "Action", &at)? {
                let at = format!("{at}/ActionMatch");
                if attribute != ACTION_ID {
                    return Err(refused(at, Loss::Code(attribute.to_owned())));
                }
                let uri = value.text.trim();
                let class = ACTIONS
                    .iter()
                    .find(|(known, _)| *known == uri)
                    .map(|(_, class)| *class)
                    .ok_or_else(|| refused(at, Loss::Code(uri.to_owned())))?;
                scope.operations.push(term(class));
            }
        }
    }
    Ok(scope)
}

fn blank_data() -> AccessDataScope {
    AccessDataScope {
        objects: None,
        templates: None,
        archetypes: None,
        categories: None,
        sensitivities: None,
        origins: None,
        recorded: None,
    }
}

fn push<T>(into: &mut Option<openehr_base::containers::NonEmptyVec<T>>, value: T) {
    let mut values = into.take().map(Vec::from).unwrap_or_default();
    values.push(value);
    *into = list(values);
}

/// Reads `condition` as the interval over the current date and time it
/// states: `and` of `dateTime-greater-than-or-equal` and
/// `dateTime-less-than-or-equal`, or either alone.
pub(super) fn validity(
    condition: Option<&Node>,
    element: &str,
) -> Result<Option<DvInterval<DvDateTime>>, Refused> {
    let Some(condition) = condition else {
        return Ok(None);
    };
    let other = || refused(element, Loss::NoDirectiveForm);
    let apply = condition.first("Apply").ok_or_else(other)?;
    let comparisons: Vec<&Node> = if apply.get("FunctionId") == Some(&format!("{FUNCTION}and")) {
        apply.all("Apply").collect()
    } else {
        vec![apply]
    };
    let (mut start, mut end) = (None, None);
    for comparison in comparisons {
        let function = comparison.get("FunctionId").unwrap_or_default();
        let current = comparison.first("Apply").filter(|inner| {
            inner.get("FunctionId") == Some(&format!("{FUNCTION}dateTime-one-and-only"))
                && inner
                    .first("EnvironmentAttributeDesignator")
                    .and_then(|d| d.get("AttributeId"))
                    == Some(CURRENT_DATE_TIME)
        });
        let bound = comparison
            .first("AttributeValue")
            .filter(|v| v.get("DataType") == Some(XS_DATE_TIME))
            .map(|v| v.text.trim().to_owned());
        match (function.strip_prefix(FUNCTION), current, bound) {
            (Some("dateTime-greater-than-or-equal"), Some(_), Some(at)) if start.is_none() => {
                start = Some(at);
            }
            (Some("dateTime-less-than-or-equal"), Some(_), Some(at)) if end.is_none() => {
                end = Some(at);
            }
            _ => return Err(other()),
        }
    }
    Ok(Some(interval(start, end)))
}

/// The `Condition` stating `validity`.
///
/// # Errors
/// [`Refused`] for an excluded bound, which the comparison FerroEHR writes
/// includes.
pub(super) fn condition(validity: &DvInterval<DvDateTime>) -> Result<Node, Refused> {
    let (start, end) = bounds(validity).ok_or_else(|| not_stated("time_validity"))?;
    let compare = |function: &str, at: &str| {
        Node::new("Apply")
            .attr("FunctionId", &format!("{FUNCTION}{function}"))
            .child(
                Node::new("Apply")
                    .attr("FunctionId", &format!("{FUNCTION}dateTime-one-and-only"))
                    .child(
                        Node::new("EnvironmentAttributeDesignator")
                            .attr("AttributeId", CURRENT_DATE_TIME)
                            .attr("DataType", XS_DATE_TIME),
                    ),
            )
            .child(
                Node::new("AttributeValue")
                    .attr("DataType", XS_DATE_TIME)
                    .with_text(at),
            )
    };
    let mut parts: Vec<Node> = Vec::new();
    if let Some(start) = start {
        parts.push(compare("dateTime-greater-than-or-equal", &start));
    }
    if let Some(end) = end {
        parts.push(compare("dateTime-less-than-or-equal", &end));
    }
    let expression = match parts.len() {
        0 => return Err(not_stated("time_validity")),
        1 => parts.remove(0),
        _ => parts.into_iter().fold(
            Node::new("Apply").attr("FunctionId", &format!("{FUNCTION}and")),
            Node::child,
        ),
    };
    Ok(Node::new("Condition").child(expression))
}

/// The attribute `attribute` of a directive has no APPC form.
pub(super) fn not_stated(attribute: &str) -> Refused {
    Refused::new(
        Format::Directive,
        attribute,
        Loss::NoFormatForm(Format::Appc),
    )
}

fn value_node(data_type: &str) -> Node {
    Node::new("AttributeValue").attr("DataType", data_type)
}

fn one_match(kind: &str, function: &str, value: Node, attribute: &str, data_type: &str) -> Node {
    Node::new(&format!("{kind}Match"))
        .attr("MatchId", function)
        .child(value)
        .child(
            Node::new(&format!("{kind}AttributeDesignator"))
                .attr("DataType", data_type)
                .attr("AttributeId", attribute),
        )
}

/// A `CV` match of `coded` on `attribute`.
fn cv_match(kind: &str, coded: &DvCodedText, attribute: &str) -> Node {
    let value = value_node(HL7_CV).child(
        Node::new("hl7:CodedValue")
            .attr("code", &coded.defining_code.code_string)
            .attr("codeSystem", &coded.defining_code.terminology_id.value)
            .attr("displayName", &coded.value),
    );
    one_match(
        kind,
        "urn:hl7-org:v3:function:CV-equal",
        value,
        attribute,
        HL7_CV,
    )
}

/// A string or URI match of `text` on `attribute`.
fn text_match(kind: &str, text: &str, attribute: &str, data_type: &str) -> Node {
    let function = if data_type == XS_ANY_URI {
        "anyURI-equal"
    } else {
        "string-equal"
    };
    one_match(
        kind,
        &format!("{FUNCTION}{function}"),
        value_node(data_type).with_text(text),
        attribute,
        data_type,
    )
}

/// The patient match the root policy set carries (APPC TF-3 5.6.2.1.1).
pub(super) fn patient(root: &str, extension: Option<&str>) -> Node {
    let mut id = Node::new("hl7:InstanceIdentifier").attr("root", root);
    if let Some(extension) = extension {
        id = id.attr("extension", extension);
    }
    Node::new("Target").child(
        Node::new("Resources").child(Node::new("Resource").child(one_match(
            "Resource",
            "urn:hl7-org:v3:function:II-equal",
            value_node(HL7_II).child(id),
            PATIENT_ID,
            HL7_II,
        ))),
    )
}

/// The subject attribute and value that name `party`.
fn party_match(party: &PartyRef) -> Result<Node, Refused> {
    let (attribute, data_type) = match party.r#type.as_str() {
        "ORGANISATION" => (ORGANIZATION_ID, XS_ANY_URI),
        "PERSON" => (SUBJECT_ID, XS_STRING),
        _ => return Err(not_stated("applies_to.party")),
    };
    if party.namespace != "demographic" {
        return Err(not_stated("applies_to.party"));
    }
    let value = match &party.id {
        ObjectId::HierObjectId(id) if attribute == ORGANIZATION_ID => {
            format!("urn:uuid:{}", id.value())
        }
        ObjectId::HierObjectId(id) => id.value().to_owned(),
        ObjectId::GenericId(id) if id.scheme == attribute => id.value.clone(),
        _ => return Err(not_stated("applies_to.party")),
    };
    Ok(text_match("Subject", &value, attribute, data_type))
}

/// The `Target` stating the scope of `directive`; `None` when it has none.
///
/// # Errors
/// [`Refused`] naming a scope attribute APPC has no attribute for:
/// `relationship`, `caller_kind`, `objects`, `templates`, `archetypes`,
/// `recorded`, the `export` operation, or purposes beside more than one
/// actor scope.
pub(super) fn write(directive: &AccessDirective) -> Result<Option<Node>, Refused> {
    let actors = members(directive.applies_to.as_ref());
    let purposes = members(directive.purposes.as_ref());
    if !purposes.is_empty() && actors.len() > 1 {
        return Err(not_stated("purposes"));
    }
    let mut subjects = Vec::new();
    for actor in actors {
        if actor.relationship.is_some() {
            return Err(not_stated("applies_to.relationship"));
        }
        if actor.caller_kind.is_some() {
            return Err(not_stated("applies_to.caller_kind"));
        }
        let mut subject = Node::new("Subject");
        if let Some(party) = &actor.party {
            subject = subject.child(party_match(party)?);
        }
        if let Some(role) = &actor.role {
            subject = subject.child(cv_match("Subject", role, ROLE));
        }
        subjects.push(subject);
    }
    if subjects.is_empty() && !purposes.is_empty() {
        subjects.push(Node::new("Subject"));
    }
    if let Some(first) = subjects.first_mut() {
        for purpose in purposes {
            first.children.push(cv_match("Subject", purpose, PURPOSE));
        }
    }
    let mut resources = Vec::new();
    for data in members(directive.data.as_ref()) {
        if data.objects.is_some() {
            return Err(not_stated("data.objects"));
        }
        if data.templates.is_some() {
            return Err(not_stated("data.templates"));
        }
        if data.archetypes.is_some() {
            return Err(not_stated("data.archetypes"));
        }
        if data.recorded.is_some() {
            return Err(not_stated("data.recorded"));
        }
        let mut resource = Node::new("Resource");
        for coded in members(data.sensitivities.as_ref()) {
            resource = resource.child(cv_match("Resource", coded, CONFIDENTIALITY_CODE));
        }
        for coded in members(data.categories.as_ref()) {
            resource = resource.child(cv_match("Resource", coded, CLASS_CODE));
        }
        for origin in members(data.origins.as_ref()) {
            resource = resource.child(text_match("Resource", origin, SOURCE_SYSTEM_ID, XS_ANY_URI));
        }
        resources.push(resource);
    }
    let mut actions = Vec::new();
    for operation in members(directive.operations.as_ref()) {
        let class = OperationClass::read(
            TERMINOLOGY_ID_ACCESS_CONTROL,
            &operation.defining_code.code_string,
        )
        .map_err(|error| {
            Refused::new(
                Format::Directive,
                "operations",
                Loss::Code(error.to_string()),
            )
        })?;
        let uri = ACTIONS
            .iter()
            .find(|(_, known)| *known == class)
            .map(|(uri, _)| *uri)
            .ok_or_else(|| not_stated("operations"))?;
        actions.push(Node::new("Action").child(text_match("Action", uri, ACTION_ID, XS_ANY_URI)));
    }
    if subjects.is_empty() && resources.is_empty() && actions.is_empty() {
        return Ok(None);
    }
    let mut target = Node::new("Target");
    for (name, group) in [
        ("Subjects", subjects),
        ("Resources", resources),
        ("Actions", actions),
    ] {
        if !group.is_empty() {
            target = target.child(group.into_iter().fold(Node::new(name), Node::child));
        }
    }
    Ok(Some(target))
}
