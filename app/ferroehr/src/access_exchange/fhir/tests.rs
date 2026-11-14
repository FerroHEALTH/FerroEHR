// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The vendored R4 and R5 `Consent` examples, the round trip of directives
//! through each release, every loss refused with its element, and the
//! extension for every attribute with no core element.

use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
use openehr_base::v1_3::base_types::identification::hier_object_id::HierObjectId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::object_ref::{ObjectRef, ObjectRefData};
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    AccessDefault, CallerKind, DirectiveKind, Effect, OperationClass, Relationship,
};

use super::export::{ExportContext, export};
use super::import::{ImportContext, foreign_actions, import};
use super::model::{Code, Document, Ref, Rule};
use super::{
    CONFIDENTIALITY, EXT_ARCHETYPE, EXT_CALLER_KIND, EXT_DIRECTIVE, EXT_EMERGENCY_OVERRIDE,
    EXT_OBJECT_REF, EXT_ORIGIN, EXT_RELATIONSHIP, EXT_ROLE, EXT_TIME_VALIDITY, Release, bundle,
    read,
};
use crate::access_exchange::rm::{directive, interval, list, opposite, term};
use crate::access_exchange::{Format, Loss, Refused};
use crate::access_log::entry::coded;
use crate::versioning::audit::dv_date_time_text;

const CONSENT_ACTION: &str = "http://terminology.hl7.org/CodeSystem/consentaction";

/// A terminology-server stub translating HL7 `consentaction` to the
/// operation classes, as a deployment's `ConceptMap` would.
fn translate(code: &Code) -> Option<OperationClass> {
    if code.system.as_deref() != Some(CONSENT_ACTION) {
        return None;
    }
    match code.code.as_deref()? {
        "access" | "use" => Some(OperationClass::Read),
        "collect" | "correct" => Some(OperationClass::Write),
        "disclose" => Some(OperationClass::Export),
        _ => None,
    }
}

fn context(default_access: AccessDefault) -> ImportContext<'static> {
    ImportContext {
        catalogue: &[],
        default_access,
        operations: &translate,
    }
}

fn export_context() -> ExportContext {
    ExportContext {
        subject: Ref {
            kind: Some("Patient".to_owned()),
            reference: Some("Patient/example".to_owned()),
            ..Ref::default()
        },
        controller: None,
        stated: "2026-11-14T10:00:00Z".to_owned(),
    }
}

fn vendored(release: Release, name: &str) -> Vec<u8> {
    let dir = match release {
        Release::R4 => "R4",
        Release::R5 => "R5",
    };
    let path = format!(
        "{}/../../docs/specs/fhir-consent/{dir}/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

/// `directives` exported one `Consent` each, through the JSON of a `Bundle`,
/// and imported again.
fn round_trip(release: Release, directives: &[AccessDirective]) -> Vec<AccessDirective> {
    let documents = export(release, directives, &export_context()).expect("exports");
    let body = bundle(release, &documents).expect("a bundle");
    let bundle: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    let mut back = Vec::new();
    for entry in bundle["entry"].as_array().expect("entries") {
        let resource = serde_json::to_vec(&entry["resource"]).expect("a resource");
        let document = read(release, &resource).expect("reads its own output");
        back.extend(
            import(release, &document, &context(AccessDefault::Restricted)).expect("imports"),
        );
    }
    back
}

/// Every vendored example imports as so many directives, which round trip,
/// or is refused at the element named.
#[test]
fn every_vendored_example_imports_or_is_refused_at_the_element_and_round_trips() {
    use Release::{R4, R5};
    let refused = |element: &str| Err(element.to_owned());
    let actor_role = "Consent.provision.actor.role";
    let cases: [(Release, &str, Result<usize, String>); 24] = [
        (R4, "consent-example.json", Ok(1)),
        (R4, "consent-example-Emergency.json", refused(actor_role)),
        (R4, "consent-example-Out.json", refused(actor_role)),
        (R4, "consent-example-grantor.json", refused(actor_role)),
        (R4, "consent-example-notAuthor.json", refused(actor_role)),
        (R4, "consent-example-notOrg.json", Ok(1)),
        (R4, "consent-example-notThem.json", Ok(1)),
        (
            R4,
            "consent-example-notThis.json",
            refused("Consent.provision.data.meaning"),
        ),
        (R4, "consent-example-notTime.json", Ok(1)),
        (R4, "consent-example-pkb.json", Ok(1)),
        (
            R4,
            "consent-example-signature.json",
            refused("Consent.provision.provision.actor.role"),
        ),
        (
            R4,
            "consent-example-smartonfhir.json",
            refused("Consent.provision.provision.class"),
        ),
        (R5, "consent-example.json", Ok(1)),
        (R5, "consent-example-CDA.json", refused("Consent.grantee")),
        (R5, "consent-example-Emergency.json", refused(actor_role)),
        (R5, "consent-example-Out.json", refused(actor_role)),
        (R5, "consent-example-grantor.json", refused(actor_role)),
        (R5, "consent-example-notAuthor.json", refused(actor_role)),
        (R5, "consent-example-notOrg.json", Ok(1)),
        (R5, "consent-example-notThem.json", Ok(1)),
        (
            R5,
            "consent-example-notThis.json",
            refused("Consent.provision.data.meaning"),
        ),
        (R5, "consent-example-notTime.json", Ok(1)),
        (R5, "consent-example-pkb.json", Ok(1)),
        (
            R5,
            "consent-example-smartonfhir.json",
            refused("Consent.provision.provision.resourceType"),
        ),
    ];
    let mut wrong = Vec::new();
    for (release, name, expected) in cases {
        let outcome = read(release, &vendored(release, name))
            .and_then(|document| import(release, &document, &context(AccessDefault::Restricted)));
        let seen = outcome
            .as_ref()
            .map(Vec::len)
            .map_err(|refusal| refusal.element.clone());
        if seen != expected {
            wrong.push(format!(
                "{release:?} {name}: expected {expected:?}, got {outcome:?}"
            ));
            continue;
        }
        if let Ok(directives) = outcome {
            assert_eq!(
                round_trip(release, &directives),
                directives,
                "{release:?} {name} round trips"
            );
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

fn party(id: &str, rm_type: &str) -> PartyRef {
    PartyRef {
        namespace: "demographic".to_owned(),
        r#type: rm_type.to_owned(),
        id: ObjectId::HierObjectId(HierObjectId::new(id).expect("an id")),
    }
}

const DOCTOR: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f81";
const NOTE: &str = "0190b3a2-5a3e-7c1d-9e2f-3b4c5d6e7f90";

/// A deny exception for one doctor reading lab data.
fn exception() -> AccessDirective {
    let mut x = directive(DirectiveKind::Consent, Effect::Deny);
    x.applies_to = list(vec![AccessActorScope {
        party: Some(party(DOCTOR, "PERSON")),
        role: None,
        relationship: None,
        caller_kind: None,
    }]);
    x.operations = list(vec![term(OperationClass::Read)]);
    x.data = list(vec![AccessDataScope {
        objects: None,
        templates: None,
        archetypes: None,
        categories: list(vec![coded(
            "http://loinc.org",
            "26436-6",
            "Laboratory studies",
        )]),
        sensitivities: None,
        origins: None,
        recorded: None,
    }]);
    x
}

/// The data scope setting every attribute.
fn every_data() -> AccessDataScope {
    AccessDataScope {
        objects: list(vec![ObjectRef::ObjectRef(ObjectRefData {
            namespace: "local".to_owned(),
            r#type: "VERSIONED_COMPOSITION".to_owned(),
            id: ObjectId::HierObjectId(HierObjectId::new(NOTE).expect("an id")),
        })]),
        templates: list(vec!["Mental health note".to_owned()]),
        archetypes: list(vec![ArchetypeId {
            value: "openEHR-EHR-COMPOSITION.progress_note.v1".to_owned(),
        }]),
        categories: list(vec![coded("ehds-categories", "mental", "mental health")]),
        sensitivities: list(vec![
            coded(CONFIDENTIALITY, "U", "U"),
            coded(CONFIDENTIALITY, "L", "L"),
            coded(CONFIDENTIALITY, "M", "M"),
            coded(CONFIDENTIALITY, "N", "N"),
            coded(CONFIDENTIALITY, "R", "R"),
            coded(
                "http://terminology.hl7.org/CodeSystem/v3-ActCode",
                "PSY",
                "PSY",
            ),
        ]),
        origins: list(vec!["lab-system".to_owned()]),
        recorded: Some(interval(Some("2026-01-01T00:00:00Z".to_owned()), None)),
    }
}

/// A directive setting every attribute the evaluator reads, and some it
/// does not.
fn full(kind: DirectiveKind, effect: Effect) -> AccessDirective {
    let mut d = directive(kind, effect);
    d.applies_to = list(vec![
        AccessActorScope {
            party: Some(party(DOCTOR, "PERSON")),
            role: Some(coded("local-roles", "GP", "general practitioner")),
            relationship: None,
            caller_kind: None,
        },
        AccessActorScope {
            party: None,
            role: None,
            relationship: Some(term(Relationship::Care)),
            caller_kind: Some(term(CallerKind::Professional)),
        },
        AccessActorScope {
            party: None,
            role: Some(coded("local-roles", "NURSE", "nurse")),
            relationship: None,
            caller_kind: None,
        },
    ]);
    d.operations = list(vec![
        term(OperationClass::Read),
        term(OperationClass::Query),
    ]);
    d.purposes = list(vec![coded(
        "http://terminology.hl7.org/CodeSystem/v3-ActReason",
        "TREAT",
        "treatment",
    )]);
    d.data = list(vec![every_data()]);
    d.time_validity = Some(interval(
        Some("2026-01-01T00:00:00Z".to_owned()),
        Some("2027-01-01T00:00:00Z".to_owned()),
    ));
    d.emergency_override = true;
    let mut x = exception();
    x.effect = term(opposite(effect));
    x.emergency_override = true;
    d.exceptions = list(vec![x]);
    d.instructed = Some(dv_date_time_text("2026-03-04T05:06:07Z".to_owned()));
    d
}

#[test]
fn directives_round_trip_through_both_releases() {
    let mut unscoped = directive(DirectiveKind::Consent, Effect::Permit);
    unscoped.exceptions = list(vec![exception()]);
    unscoped.time_validity = Some(interval(Some("2026-01-01T00:00:00Z".to_owned()), None));
    let mut opt_out = directive(DirectiveKind::OptOut, Effect::Deny);
    opt_out.emergency_override = true;
    let directives = vec![
        unscoped,
        opt_out,
        full(DirectiveKind::Restriction, Effect::Deny),
        full(DirectiveKind::Consent, Effect::Permit),
        full(DirectiveKind::Delegation, Effect::Permit),
    ];
    for release in [Release::R4, Release::R5] {
        assert_eq!(round_trip(release, &directives), directives, "{release:?}");
    }
    let mut two = directive(DirectiveKind::Consent, Effect::Permit);
    let mut second = exception();
    second.operations = list(vec![term(OperationClass::Export)]);
    two.exceptions = list(vec![exception(), second]);
    assert_eq!(round_trip(Release::R5, &[two.clone()]), vec![two.clone()]);
    assert_eq!(
        export(Release::R4, &[two], &export_context())
            .and_then(|documents| bundle(Release::R4, &documents))
            .map(|_| ()),
        Err(Refused::new(
            Format::Directive,
            "exceptions",
            Loss::NoFormatForm(Format::FhirR4)
        )),
        "R4 holds one root provision"
    );
}

/// Every extension URL `document` carries, at any level.
fn urls(document: &Document) -> Vec<String> {
    fn rule_urls(rules: &[Rule], out: &mut Vec<String>) {
        for rule in rules {
            out.extend(rule.extensions.iter().map(|e| e.url.clone()));
            for actor in &rule.actors {
                out.extend(actor.extensions.iter().map(|e| e.url.clone()));
            }
            for data in &rule.data {
                out.extend(data.extensions.iter().map(|e| e.url.clone()));
            }
            rule_urls(&rule.rules, out);
        }
    }
    let mut out: Vec<String> = document.extensions.iter().map(|e| e.url.clone()).collect();
    rule_urls(&document.rules, &mut out);
    out
}

#[test]
fn export_writes_the_extension_of_every_attribute_with_no_core_element() {
    let scoped = full(DirectiveKind::Restriction, Effect::Deny);
    for release in [Release::R4, Release::R5] {
        let documents =
            export(release, std::slice::from_ref(&scoped), &export_context()).expect("exports");
        let seen = urls(&documents[0]);
        for url in [
            EXT_DIRECTIVE,
            EXT_EMERGENCY_OVERRIDE,
            EXT_ROLE,
            EXT_RELATIONSHIP,
            EXT_CALLER_KIND,
            EXT_ARCHETYPE,
            EXT_ORIGIN,
            EXT_OBJECT_REF,
        ] {
            assert!(seen.iter().any(|u| u == url), "{release:?} writes {url}");
        }
    }
    let mut unscoped = directive(DirectiveKind::Consent, Effect::Deny);
    unscoped.time_validity = Some(interval(None, Some("2027-01-01T00:00:00Z".to_owned())));
    let r4 = export(Release::R4, &[unscoped.clone()], &export_context()).expect("exports");
    assert!(urls(&r4[0]).iter().any(|u| u == EXT_TIME_VALIDITY));
    let r5 = export(Release::R5, &[unscoped], &export_context()).expect("exports");
    assert!(r5[0].period.is_some(), "R5 states it as Consent.period");
}

#[test]
fn export_refuses_what_a_consent_cannot_state() {
    let mut two_scopes = exception();
    two_scopes.data = list(vec![every_data(), every_data()]);
    let mut excluded = directive(DirectiveKind::Consent, Effect::Deny);
    let mut bound = interval(Some("2026-01-01T00:00:00Z".to_owned()), None);
    bound.lower_included = false;
    excluded.time_validity = Some(bound);
    let mut gap = exception();
    gap.data = list(vec![AccessDataScope {
        objects: None,
        templates: None,
        archetypes: None,
        categories: None,
        sensitivities: list(vec![coded(CONFIDENTIALITY, "R", "R")]),
        origins: None,
        recorded: None,
    }]);
    for (directive, attribute) in [
        (two_scopes, "data"),
        (excluded, "time_validity"),
        (gap, "data.sensitivities"),
    ] {
        for release in [Release::R4, Release::R5] {
            assert_eq!(
                export(release, std::slice::from_ref(&directive), &export_context()).map(|_| ()),
                Err(Refused::new(
                    Format::Directive,
                    attribute,
                    Loss::NoFormatForm(release.format())
                )),
                "{release:?} {attribute}"
            );
        }
    }
}

/// `consent` read and imported as `release` into an EHR of `default_access`.
fn imported(
    release: Release,
    consent: &serde_json::Value,
    default_access: AccessDefault,
) -> Result<Vec<AccessDirective>, Refused> {
    let body = serde_json::to_vec(consent).expect("JSON");
    let document = read(release, &body)?;
    import(release, &document, &context(default_access))
}

fn r4(provision: &serde_json::Value, policy_rule: &str) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Consent",
        "status": "active",
        "scope": { "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/consentscope",
                                "code": "patient-privacy" }] },
        "category": [{ "coding": [{ "system": "http://loinc.org", "code": "59284-0" }] }],
        "policyRule": { "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/v3-ActCode",
                                     "code": policy_rule }] },
        "provision": provision,
    })
}

fn r5(provisions: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "resourceType": "Consent",
        "status": "active",
        "decision": "deny",
        "provision": provisions,
    })
}

fn actor() -> serde_json::Value {
    serde_json::json!([{
        "role": { "coding": [{ "system": "http://terminology.hl7.org/CodeSystem/v3-ParticipationType",
                               "code": "PRCP" }] },
        "reference": { "reference": "Organization/f001" },
    }])
}

/// One import refused at `element` for `loss`.
struct Case {
    release: Release,
    consent: serde_json::Value,
    default_access: AccessDefault,
    element: &'static str,
    loss: Loss,
}

fn case(release: Release, consent: serde_json::Value, element: &'static str, loss: Loss) -> Case {
    Case {
        release,
        consent,
        default_access: AccessDefault::Restricted,
        element,
        loss,
    }
}

/// The losses of the provision structure.
fn structure_cases() -> Vec<Case> {
    use Release::{R4, R5};
    let deep = serde_json::json!({ "actor": actor(), "provision": [{ "actor": actor(),
        "provision": [{ "actor": actor() }] }] });
    let mut cases = vec![
        case(
            R4,
            r4(&deep, "OPTIN"),
            "Consent.provision.provision.provision",
            Loss::TooDeep,
        ),
        case(
            R5,
            r5(
                &serde_json::json!([{ "actor": actor(), "provision": [{ "actor": actor(),
                "provision": [{ "actor": actor() }] }] }]),
            ),
            "Consent.provision.provision.provision",
            Loss::TooDeep,
        ),
    ];
    for meaning in ["related", "dependents", "authoredby"] {
        let data = serde_json::json!({ "data": [{ "meaning": meaning,
            "reference": { "reference": "DocumentReference/1" } }] });
        cases.push(case(
            R4,
            r4(&data, "OPTIN"),
            "Consent.provision.data.meaning",
            Loss::Code(meaning.to_owned()),
        ));
    }
    let expression = serde_json::json!([{ "expression": { "language": "text/fhirpath",
        "expression": "true" } }]);
    cases.push(case(
        R5,
        r5(&expression),
        "Consent.provision.expression",
        Loss::NoDirectiveForm,
    ));
    let resource_type = serde_json::json!([{ "resourceType": [{ "system":
        "http://hl7.org/fhir/resource-types", "code": "Observation" }] }]);
    cases.push(case(
        R5,
        r5(&resource_type),
        "Consent.provision.resourceType",
        Loss::NoDirectiveForm,
    ));
    let mut basis = r5(&serde_json::json!([{ "actor": actor() }]));
    basis["policyBasis"] = serde_json::json!({ "url": "https://example.org/policy" });
    cases.push(case(
        R5,
        basis,
        "Consent.policyBasis.url",
        Loss::NotInCatalogue("https://example.org/policy".to_owned()),
    ));
    let mut no_decision = r5(&serde_json::json!([{ "actor": actor() }]));
    no_decision
        .as_object_mut()
        .expect("an object")
        .remove("decision");
    cases.push(case(R5, no_decision, "Consent.decision", Loss::NoBase));
    cases.push(case(
        R4,
        r4(&serde_json::json!({ "actor": actor() }), "urn:oid:1.2.3"),
        "Consent.policyRule",
        Loss::NotInCatalogue(
            "http://terminology.hl7.org/CodeSystem/v3-ActCode|urn:oid:1.2.3".to_owned(),
        ),
    ));
    let same = serde_json::json!({ "actor": actor(), "provision": [{ "type": "permit",
        "actor": actor() }] });
    cases.push(case(
        R4,
        r4(&same, "OPTOUT"),
        "Consent.provision.provision.type",
        Loss::NotException,
    ));
    let nested = serde_json::json!({ "actor": actor(), "provision": [{ "actor": actor() }] });
    cases.push(Case {
        default_access: AccessDefault::Open,
        ..case(
            R4,
            r4(&nested, "OPTOUT"),
            "Consent.policyRule",
            Loss::DenyOverOpen,
        )
    });
    cases
}

/// The losses of codes and extensions.
fn code_cases() -> Vec<Case> {
    use Release::R4;
    let mut cases = Vec::new();
    let mut custodian = actor();
    custodian[0]["role"]["coding"][0]["code"] = serde_json::json!("CST");
    cases.push(case(
        R4,
        r4(&serde_json::json!({ "actor": custodian }), "OPTIN"),
        "Consent.provision.actor.role",
        Loss::Code("http://terminology.hl7.org/CodeSystem/v3-ParticipationType|CST".to_owned()),
    ));
    let action = serde_json::json!({ "action": [{ "coding": [{ "system": "urn:example:actions",
        "code": "print" }] }] });
    cases.push(case(
        R4,
        r4(&action, "OPTIN"),
        "Consent.provision.action",
        Loss::Untranslated("urn:example:actions|print".to_owned()),
    ));
    let mut inactive = r4(&serde_json::json!({ "actor": actor() }), "OPTIN");
    inactive["status"] = serde_json::json!("inactive");
    cases.push(case(
        R4,
        inactive,
        "Consent.status",
        Loss::Code("inactive".to_owned()),
    ));
    let mut research = r4(&serde_json::json!({ "actor": actor() }), "OPTIN");
    research["scope"]["coding"][0]["code"] = serde_json::json!("research");
    cases.push(case(
        R4,
        research,
        "Consent.scope",
        Loss::Code("http://terminology.hl7.org/CodeSystem/consentscope|research".to_owned()),
    ));
    let modifier = serde_json::json!({ "actor": actor(), "modifierExtension": [{
        "url": "urn:example:not", "valueBoolean": true }] });
    cases.push(case(
        R4,
        r4(&modifier, "OPTIN"),
        "Consent.provision.modifierExtension",
        Loss::NoDirectiveForm,
    ));
    let unknown = serde_json::json!({ "actor": actor(), "extension": [{
        "url": "urn:example:unknown", "valueBoolean": true }] });
    cases.push(case(
        R4,
        r4(&unknown, "OPTIN"),
        "Consent.provision.extension",
        Loss::Code("urn:example:unknown".to_owned()),
    ));
    cases
}

#[test]
fn each_loss_is_refused_on_import_with_the_element_named() {
    let cases = structure_cases().into_iter().chain(code_cases());
    for Case {
        release,
        consent,
        default_access,
        element,
        loss,
    } in cases
    {
        assert_eq!(
            imported(release, &consent, default_access),
            Err(Refused::new(release.format(), element, loss)),
            "{release:?} {element}"
        );
    }
}

#[test]
fn a_foreign_action_is_named_for_translation_and_a_label_covers_the_levels_below() {
    let body = vendored(Release::R4, "consent-example-pkb.json");
    let document = read(Release::R4, &body).expect("reads");
    let actions = foreign_actions(&document);
    assert_ne!(actions, Vec::new());
    assert!(
        actions
            .iter()
            .all(|a| a.system.as_deref() == Some(CONSENT_ACTION))
    );
    let directives =
        import(Release::R4, &document, &context(AccessDefault::Restricted)).expect("imports");
    let scoped = directives
        .iter()
        .find(|d| d.data.is_some())
        .expect("the scoped directive");
    let scope = &scoped.data.as_ref().expect("a scope")[0];
    let levels: Vec<&str> = scope
        .sensitivities
        .as_ref()
        .expect("labels")
        .iter()
        .map(|c| c.defining_code.code_string.as_str())
        .collect();
    assert_eq!(
        levels,
        ["U", "L", "M", "N"],
        "N covers itself and every level below"
    );
}
