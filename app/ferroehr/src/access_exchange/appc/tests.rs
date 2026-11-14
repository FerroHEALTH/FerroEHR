// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The APPC example document, the round trip of directives, and every loss
//! refused with its element.

use openehr_base::v1_3::base_types::identification::generic_id::GenericId;
use openehr_base::v1_3::base_types::identification::object_id::ObjectId;
use openehr_base::v1_3::base_types::identification::party_ref::PartyRef;
use openehr_rm::draft::access_control::access_control::access_actor_scope::AccessActorScope;
use openehr_rm::draft::access_control::access_control::access_data_scope::AccessDataScope;
use openehr_rm::draft::access_control::access_control::access_directive::AccessDirective;
use openehr_rm::draft::access_control::terminology::{
    CallerKind, DirectiveKind, Effect, OperationClass,
};
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;

use super::target::ORGANIZATION_ID;
use super::{ExportContext, ImportContext, export, import};
use crate::access_exchange::rm::{directive, interval, list, opposite, term};
use crate::access_exchange::{Format, Loss, Refused};
use crate::access_log::entry::coded;

/// The example document of APPC TF-3 5.6.2.1.1.2, transcribed from the
/// vendored supplement (`docs/specs/ihe-consents/www.ihe.net/uploadedFiles/
/// Documents/ITI/IHE_ITI_Suppl_APPC.pdf`, page 35) without its line numbers,
/// page furniture and `xsi:schemaLocation`, since a test cannot read the PDF.
const EXAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<PolicySet PolicySetId="urn:uuid:e3585197-9e3d-4ca3-9583-4540a3a5b64b"
  PolicyCombiningAlgId="urn:oasis:names:tc:xacml:1.0:policy-combining-algorithm:deny-overrides"
  xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"
  xmlns:hl7="urn:hl7-org:v3"
  xmlns="urn:oasis:names:tc:xacml:2.0:policy:schema:os">
  <Description>The patient agrees to grant access to the identified
facility. The extent of access is defined by the referenced policy.
  </Description>
  <Target>
    <Subjects>
      <Subject>
        <SubjectMatch MatchId="urn:oasis:names:tc:xacml:1.0:function:anyURI-equal">
          <AttributeValue DataType="http://www.w3.org/2001/XMLSchema#anyURI">
            urn:oid:2.999.2.1.1.35
          </AttributeValue>
          <SubjectAttributeDesignator DataType="http://www.w3.org/2001/XMLSchema#anyURI"
            AttributeId="urn:oasis:names:tc:xspa:1.0:subject:organization-id" />
        </SubjectMatch>
      </Subject>
    </Subjects>
    <Resources>
      <Resource>
        <ResourceMatch MatchId="urn:hl7-org:v3:function:II-equal">
          <AttributeValue DataType="urn:hl7-org:v3#II">
            <hl7:InstanceIdentifier root="2.999.1.1.1" extension="78901234" />
          </AttributeValue>
          <ResourceAttributeDesignator DataType="urn:hl7-org:v3#II"
            AttributeId="urn:ihe:iti:ser:2016:patient-id" />
        </ResourceMatch>
      </Resource>
    </Resources>
  </Target>
  <PolicySetIdReference>
    urn:example:policy:extensive-access
  </PolicySetIdReference>
</PolicySet>"#;

/// The foundational policy the example references.
fn catalogue() -> Vec<AccessDirective> {
    let mut entry = directive(DirectiveKind::Consent, Effect::Permit);
    entry.operations = list(vec![
        term(OperationClass::Read),
        term(OperationClass::Query),
    ]);
    entry.identifiers = list(vec![DvIdentifier {
        issuer: None,
        assigner: None,
        id: "urn:example:policy:extensive-access".to_owned(),
        r#type: None,
    }]);
    vec![entry]
}

fn context() -> ExportContext<'static> {
    ExportContext {
        patient_root: "2.999.1.1.1",
        patient_extension: Some("78901234"),
        description: "The access directives of one EHR.",
    }
}

/// `directives` written as one document and read again.
fn round_trip(directives: &[AccessDirective]) -> Vec<AccessDirective> {
    let text = export(directives, &context()).expect("exports");
    import(&text, &ImportContext { catalogue: &[] }).expect("reads its own output")
}

#[test]
fn the_example_reads_as_the_referenced_policy_narrowed_to_the_facility_and_round_trips() {
    let catalogue = catalogue();
    let imported = import(
        EXAMPLE,
        &ImportContext {
            catalogue: &catalogue,
        },
    )
    .expect("imports");
    let mut expected = catalogue[0].clone();
    expected.applies_to = list(vec![AccessActorScope {
        party: Some(PartyRef {
            namespace: "demographic".to_owned(),
            r#type: "ORGANISATION".to_owned(),
            id: ObjectId::GenericId(GenericId {
                value: "urn:oid:2.999.2.1.1.35".to_owned(),
                scheme: ORGANIZATION_ID.to_owned(),
            }),
        }),
        role: None,
        relationship: None,
        caller_kind: None,
    }]);
    assert_eq!(imported, vec![expected]);
    assert_eq!(round_trip(&imported), imported);
}

fn lab() -> AccessDataScope {
    AccessDataScope {
        objects: None,
        templates: None,
        archetypes: None,
        categories: list(vec![coded(
            "2.16.840.1.113883.6.1",
            "26436-6",
            "Laboratory studies",
        )]),
        sensitivities: list(vec![coded("2.16.840.1.113883.5.25", "R", "restricted")]),
        origins: list(vec!["urn:oid:2.999.3".to_owned()]),
        recorded: None,
    }
}

#[test]
fn directives_round_trip_through_a_privacy_consent_document() {
    let mut permit = directive(DirectiveKind::Consent, Effect::Permit);
    permit.applies_to = list(vec![AccessActorScope {
        party: None,
        role: Some(coded("2.16.840.1.113883.6.96", "46255001", "pharmacist")),
        relationship: None,
        caller_kind: None,
    }]);
    permit.purposes = list(vec![coded("1.0.14265.1", "1", "clinical care")]);
    permit.operations = list(vec![term(OperationClass::Read)]);
    permit.time_validity = Some(interval(
        Some("2026-01-01T00:00:00Z".to_owned()),
        Some("2027-01-01T00:00:00Z".to_owned()),
    ));
    let mut x = directive(DirectiveKind::Consent, Effect::Deny);
    x.data = list(vec![lab()]);
    x.time_validity = Some(interval(None, Some("2026-06-01T00:00:00Z".to_owned())));
    permit.exceptions = list(vec![x]);
    let mut restriction = directive(DirectiveKind::Restriction, Effect::Deny);
    restriction.emergency_override = true;
    restriction.data = list(vec![lab()]);
    restriction.instructed = Some(crate::versioning::audit::dv_date_time_text(
        "2026-02-03T04:05:06Z".to_owned(),
    ));
    let mut opt_out = directive(DirectiveKind::OptOut, Effect::Deny);
    opt_out.emergency_override = true;
    let directives = vec![permit, restriction, opt_out];
    assert_eq!(round_trip(&directives), directives);
}

#[test]
fn each_loss_is_refused_on_import_with_the_element_named() {
    let catalogue = catalogue();
    let read = |text: &str| {
        import(
            text,
            &ImportContext {
                catalogue: &catalogue,
            },
        )
    };
    let permit_overrides = EXAMPLE.replace(
        "policy-combining-algorithm:deny-overrides",
        "policy-combining-algorithm:permit-overrides",
    );
    assert_eq!(
        read(&permit_overrides),
        Err(Refused::new(
            Format::Appc,
            "PolicySet/@PolicyCombiningAlgId",
            Loss::Code(
                "urn:oasis:names:tc:xacml:1.0:policy-combining-algorithm:permit-overrides"
                    .to_owned()
            )
        ))
    );
    let unknown = EXAMPLE.replace(
        "urn:example:policy:extensive-access\n",
        "urn:example:policy:unknown\n",
    );
    assert_eq!(
        read(&unknown),
        Err(Refused::new(
            Format::Appc,
            "PolicySet/PolicySetIdReference",
            Loss::NotInCatalogue("urn:example:policy:unknown".to_owned())
        ))
    );
    let organization_name = EXAMPLE.replace(
        "urn:oasis:names:tc:xspa:1.0:subject:organization-id",
        "urn:oasis:names:tc:xspa:1.0:subject:organization",
    );
    assert_eq!(
        read(&organization_name),
        Err(Refused::new(
            Format::Appc,
            "PolicySet/Target/Subjects/Subject/SubjectMatch",
            Loss::Code("urn:oasis:names:tc:xspa:1.0:subject:organization".to_owned())
        ))
    );
    let mut deny = directive(DirectiveKind::Consent, Effect::Deny);
    let mut x = directive(DirectiveKind::Consent, Effect::Permit);
    x.data = list(vec![lab()]);
    deny.exceptions = list(vec![x]);
    let written = export(&[deny], &context()).expect("exports");
    let mismatched = written.replace(
        "rule-combining-algorithm:permit-overrides",
        "rule-combining-algorithm:deny-overrides",
    );
    assert_eq!(
        read(&mismatched),
        Err(Refused::new(
            Format::Appc,
            "Policy/@RuleCombiningAlgId",
            Loss::Code(
                "urn:oasis:names:tc:xacml:1.0:rule-combining-algorithm:deny-overrides".to_owned()
            )
        ))
    );
    let mut dated = directive(DirectiveKind::Consent, Effect::Permit);
    dated.time_validity = Some(interval(Some("2026-01-01T00:00:00Z".to_owned()), None));
    let condition = export(&[dated], &context()).expect("exports").replace(
        "urn:oasis:names:tc:xacml:1.0:environment:current-dateTime",
        "urn:example:environment:other",
    );
    assert_eq!(
        read(&condition),
        Err(Refused::new(
            Format::Appc,
            "Policy/Rule/Condition",
            Loss::NoDirectiveForm
        ))
    );
}

#[test]
fn export_refuses_a_scope_appc_has_no_attribute_for() {
    let not_stated = |attribute: &str| {
        Refused::new(
            Format::Directive,
            attribute,
            Loss::NoFormatForm(Format::Appc),
        )
    };
    let mut kind = directive(DirectiveKind::Restriction, Effect::Deny);
    kind.applies_to = list(vec![AccessActorScope {
        party: None,
        role: None,
        relationship: None,
        caller_kind: Some(term(CallerKind::Professional)),
    }]);
    let mut templated = directive(DirectiveKind::Consent, Effect::Deny);
    let mut scope = lab();
    scope.templates = list(vec!["Mental health note".to_owned()]);
    templated.data = list(vec![scope]);
    let mut exported = directive(DirectiveKind::Consent, Effect::Deny);
    exported.operations = list(vec![term(OperationClass::Export)]);
    let mut bare = directive(DirectiveKind::Consent, Effect::Permit);
    bare.exceptions = list(vec![directive(
        DirectiveKind::Consent,
        opposite(Effect::Permit),
    )]);
    for (d, attribute) in [
        (kind, "applies_to.caller_kind"),
        (templated, "data.templates"),
        (exported, "operations"),
        (bare, "exceptions"),
    ] {
        assert_eq!(
            export(&[d], &context()),
            Err(not_stated(attribute)),
            "{attribute}"
        );
    }
}
