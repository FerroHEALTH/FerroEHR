### 3:5.7.1 Scope

The AuditEvent resource is used to record that an auditable event has happened. These auditable events are usually security or privacy relevant, but may be for other purposes. The profiles of AuditEvent here are to support security and/or privacy use-cases. These profiles of AuditEvent can be used as is, or may be used to derive more specific profiles of AuditEvent for more specific use-cases. 

### 3:5.7.2 Content

The following AuditEvent patterns are defined:
- [RESTful activities](content.html#3573-restful-activities)
- [SAML Security Token](content.html#3574-saml-security-token)
- [OAuth Security Token](content.html#3575-oauth-security-token)
- [Consent Authorized Decision Audit Message](content.html#3576-consent-authorized-decision-audit-message)
- [Privacy Disclosure Audit Message](content.html#3577-privacy-disclosure-audit-message)

### 3:5.7.3 RESTful activities

Create, Update, and Query follow a similar pattern where the initiator (client) of the RESTful interaction is identified as the Source Role ID and the responder (server) is the Destination Role ID. Read pattern with the (client) of the RESTful interaction, which will receive the data, is considered the Destination Role ID and the responder (server) is the Source Role ID. This pattern is found in both DICOM and XDS. Delete uses of a pattern with (client) of the RESTful interaction is indicated as the Application and (server) of the RESTful interaction is indicated as the Custodian.

When a FHIR RESTful interaction happens, the following AuditEvent patterns can be used. These AuditEvent patterns include a `user` slice that is minimally populated. These AuditEvent patterns can be combined with the Security Token Use pattern to record one AuditEvent that contains the details of the security context and the FHIR RESTful context.

There are two sets of profiles distinguished by [Patient as a subject](volume-1.html#152416-patient-as-a-subject) being mandated to be populated. 

 REST event | basic profile | with patient | examples
-----------|---------------|-----------------------------
Create (create) | [Create](StructureDefinition-IHE.BasicAudit.Create.html) | [PatientCreate](StructureDefinition-IHE.BasicAudit.PatientCreate.html) | [examples](StructureDefinition-IHE.BasicAudit.PatientCreate-examples.html)
Read (read and vread) | [Read](StructureDefinition-IHE.BasicAudit.Read.html) | [PatientRead](StructureDefinition-IHE.BasicAudit.PatientRead.html) | [examples](StructureDefinition-IHE.BasicAudit.PatientRead-examples.html)
Update (update, patch) | [Update](StructureDefinition-IHE.BasicAudit.Update.html) | [PatientUpdate](StructureDefinition-IHE.BasicAudit.PatientUpdate.html) | [examples](StructureDefinition-IHE.BasicAudit.PatientUpdate-examples.html)
Delete (delete) | [Delete](StructureDefinition-IHE.BasicAudit.Delete.html) |  [PatientDelete](StructureDefinition-IHE.BasicAudit.PatientDelete.html) | [examples](StructureDefinition-IHE.BasicAudit.PatientDelete-examples.html)
Execute (search and query) | [Query](StructureDefinition-IHE.BasicAudit.Query.html) | [PatientQuery](StructureDefinition-IHE.BasicAudit.PatientQuery.html) | [examples](StructureDefinition-IHE.BasicAudit.PatientQuery-examples.html)
{:.grid}

An example of an auditable event being recorded by the client and server is represented by the Create examples. 

#### 3:5.7.3.1 X-Request-Id header 

Where it is known that an http RESTful transaction included an X-Request-Id, that value should be recorded in an .entity dedicated to X-Request-Id. This ID can be used to correlated AuditEvents from client and server, and may aid with correlation on further activities recorded caused by the transaction. This means that the .entity holding the X-Request-Id may appear in AuditEvents beyond those defined here.

### 3:5.7.4 SAML Security Token

This section only applies to auditing agents that have access to the content of the SAML Security Token. This section is only concerned with the AuditEvent profiling to use when an event is attributable to an agent defined in a SAML Security Token. This does not cover how the SAML Token was created, acquired, authorized, or managed. See [IHE-XUA](https://profiles.ihe.net/ITI/TF/Volume1/ch-13.html).  This specification presumes that the interactions to obtain the "SAML Security Token" were themselves auditable events and properly recorded in the audit log. 

In this section, we use the term "SAML Security Token" in a general sense to refer to either (a) an XUA SAML token being used in an [\[ITI-40\]](https://profiles.ihe.net/ITI/TF/Volume2/ITI-40.html#3.40) or (b) any other SAML token used to secure an Interoperability transaction.

There is documentation of the audit event details to be added to a grouped transaction audit event log in [XUA Security Considerations 2:3.40.4.2](https://profiles.ihe.net/ITI/TF/Volume2/ITI-40.html#3.40.4.2); however, this does not explain how this should be recorded in a FHIR AuditEvent. There are two patterns defined here: minimal and comprehensive. Where minimal presumes that when the audit log is used, the system using the AuditEvent has access to the security infrastructure registry and logs to lookup the identifiers, thus the details that could be looked up later are not replicated in the AuditEvent. Comprehensive presumes that there is no access to lookup these details, thus preserving as much of the Security Token into the AuditEvent as is reasonable and useful.

#### 3:5.7.4.1 SAML - Minimal AuditEvent record

The Minimal AuditEvent pattern defined here is not the same as the one defined in [XUA](https://profiles.ihe.net/ITI/TF/Volume1/ch-13.html), mostly due to the more expressive and coded nature of the FHIR AuditEvent over the DICOM AuditMessage.

The Minimal AuditEvent pattern preserves the SAML ID so that the contents of the SAML assertion can be retrieved from the SAML Identity Provider (IdP) when such access is available. The Minimal AuditEvent pattern preserves the Issuer and Subject NameID in the who.Identifier, and any purposeOfUse into the purposeOfUse element.

Note: that XUA recommends alias be filled with SPProvidedID which is not defined in XUA and is found to be deprecated in many SAML specifications. So this specification does not record or refer to the SPProvidedId.

- [StructureDefinition profile of Basic AuditEvent pattern for when activity was authorized by an SAML access token](StructureDefinition-IHE.BasicAudit.SAMLaccessTokenUse.Minimal.html)
  - [examples](StructureDefinition-IHE.BasicAudit.SAMLaccessTokenUse.Minimal-examples.html)

#### 3:5.7.4.2 SAML - Comprehensive AuditEvent record

This pattern preserves most SAML attributes in the AuditEvent. Not all are preserved as some attributes are proven during the SAML token validation and thus carry no further information useful in an AuditEvent (e.g., not before). 

Local policy may choose to record less than are defined here, or may choose to record more. 

For those using SAML beyond XUA, there is no specific guidance here.

- [StructureDefinition profile of Basic AuditEvent pattern for SAML Comprehensive](StructureDefinition-IHE.BasicAudit.SAMLaccessTokenUse.Comprehensive.html)
  - [examples](StructureDefinition-IHE.BasicAudit.SAMLaccessTokenUse.Comprehensive-examples.html)

#### 3:5.7.4.3 SAML mapping to AuditEvent

The following table uses a short-hand for the SAML fields and FHIR AuditEvent elements to keep the table compact. It is presumed the reader can understand the SAML field and the FHIR AuditEvent element given. Note the `~` character represents attributes under the SAML `AttributeStatement`. There are SAML fields defined that are not shown in the following table, these SAML fields need not be preserved in the AuditEvent.

| SAML field                   | Comprehensive AuditEvent     | Minimal AuditEvent           |
|------------------------------|-----------------------------------|-----------------------------------|
| ID                           | agent[user].policy | agent[user].policy |
| Issuer                       | agent[user].who.identifier.system | agent[user].who.identifier.system |
| Subject.NameID               | agent[user].who.identifier.value  | agent[user].who.identifier.value  |
| AuthnContextClassRef         | agent[user].extension[assuranceLevel]  |
| ~subject:subject-id          | agent[user].extension[otherId][subject-id].identifier.value |
| ~subject:npi                 | agent[user].extension[otherId][npi].identifier.value |
| ~subject:provider-identifier | agent[user].extension[otherId][provider-id].identifier.value |
| ~subject:organization        | agent[userorg].who.display |
| ~subject:organization-id     | agent[userorg].who.identifier.value |
| ~subject:role                | agent[user].role | agent[user].role |
| ~subject:purposeofuse        | agent[user].purposeOfUse | agent[user].purposeOfUse
| ~bppc:2007:docid             | entity[consent].what.identifier.value |
| ~xua:2012:acp                | entity[consent].detail[acp].valueString |
| ~resource:resource-id        | entity[consent].detail[patient-id].valueString |
| ~homeCommunityId             | entity[consent].what.identifier.assigner.identifier.value | 
{:.grid}

Other SAML attributes may be defined, but this specification focuses on XUA attributes only.

### 3:5.7.5 OAuth Security Token

This section is only concerned with the AuditEvent profiling to use when an event is attributable to an agent defined in an OAuth Security Token. This does not cover how the oAuth Token was created, acquired, authorized, or managed. See [IUA](https://profiles.ihe.net/ITI/IUA/index.html).  This specification presumes that the interactions to obtain the "Oauth Security Token" were themselves auditable events and properly recorded in the audit log. 

In this section we use the term "OAuth Security Token" in a general sense to refer to either (a) an [IUA](https://profiles.ihe.net/ITI/IUA/index.html) OAuth Access Token being used in an [\[ITI-72\]](https://profiles.ihe.net/ITI/IUA/index.html#372-incorporate-access-token-iti-72) or (b) any other OAuth Access Token used to secure an Interoperability transaction (e.g., [SMART Backend Services](https://hl7.org/fhir/uv/bulkdata/authorization/index.html#presenting-an-access-token-to-fhir-api), or [SMART App Launch Framework](http://hl7.org/fhir/smart-app-launch/index.html#step-4-app-accesses-clinical-data-via-fhir-api), or other).

There are three patterns defined: opaque, minimal, and comprehensive. 
- Where opaque is used when the client and/or server has only an opaque token (encrypted JWT) where no further details are known. There is still a need to include some evidence in the AuditEvent to tie this audit log entry with a specific token, but the whole token should not be recorded for security reasons. There are introspection functions often available, but often use of this impacts performance.
- Where minimal presumes that when the audit log is used, the system using the AuditEvent has access to the security infrastructure registry and logs to lookup the identifiers, thus the details that could be looked up later are not replicated in the AuditEvent. 
- Comprehensive presumes that there is no access to lookup these details, thus preserving as much of the Security Token into the AuditEvent as is reasonable and useful.

Given that IHE has the [IUA Profile](https://profiles.ihe.net/ITI/IUA/index.html), and has [Security Audit Considerations](https://profiles.ihe.net/ITI/IUA/index.html#37251-security-audit-considerations), the AuditEvent specification here will focus on IUA interactions. The profiling AuditEvent defined here is the AuditEvent that the Client and Server would record when using [IUA](https://profiles.ihe.net/ITI/IUA/index.html) with the [ITI TF-2: 3.72 Incorporate Access Token \[ITI-72\]](https://profiles.ihe.net/ITI/IUA/index.html#372-incorporate-access-token-iti-72) to secure some RESTful transaction. The RESTful transaction is not defined here, just the additional AuditEvent element details that would be added to the AuditEvent for the RESTful transaction being secured (see [RESTful activities](content.html#3573-restful-activities) for general purpose logging).

The [IUA](https://profiles.ihe.net/ITI/IUA/index.html) Profile is used here as a proxy for all oAuth specifications. [IUA](https://profiles.ihe.net/ITI/IUA/index.html) is used here because IHE has direct access and has defined fields. The Minimal AuditEvent pattern defined here is not the same as the one defined in [IUA](https://profiles.ihe.net/ITI/IUA/index.html), mostly due to the more expressive and coded nature of the FHIR AuditEvent vs. the DICOM AuditMessage.

The Client and Server are both encouraged to log an AuditEvent covering any security relevant event, and to enhance that AuditEvent with one of the following patterns when an oAuth token is associated with authorizing (or forbidding) the event.

#### 3:5.7.5.1 oAuth - Opaque AuditEvent record

The Opaque AuditEvent profile would be used when the token is opaque (usually encrypted JWT). The token should be shortened to not more than 32 characters, and those characters should be from the end of the token to assure the most entropy. The presumption is that the many AuditEvent(s) can be correlated to all be related to the same security token given this shortened token string.

Other elements may be recorded following the Minimal or Comprehensive patterns.

- [StructureDefinition profile of Basic AuditEvent pattern for oAuth Opaque](StructureDefinition-IHE.BasicAudit.OAUTHaccessTokenUse.Opaque.html)
  - [examples](StructureDefinition-IHE.BasicAudit.OAUTHaccessTokenUse.Opaque-examples.html)

#### 3:5.7.5.2 oAuth - Minimal AuditEvent record

The Minimal AuditEvent pattern would be used when the JWT token is available at the time the AuditEvent is recorded, but where minimal oAuth details are desired in the AuditEvent log. 

The Minimal AuditEvent pattern defined here is not the same as the one defined in [\[ITI-72\] Security Audit Considerations](https://profiles.ihe.net/ITI/IUA/index.html#37251-security-audit-considerations), mostly due to the more expressive and coded nature of the FHIR AuditEvent over the DICOM AuditMessage. This AuditEvent pattern is also recommended for both client and server to the extent possible.

The Minimal AuditEvent record recommends using the Opaque AuditEvent record

- [StructureDefinition profile of Basic AuditEvent pattern for when activity was authorized by an oAuth security token](StructureDefinition-IHE.BasicAudit.OAUTHaccessTokenUse.Minimal.html)
  - [examples](StructureDefinition-IHE.BasicAudit.OAUTHaccessTokenUse.Minimal-examples.html)

#### 3:5.7.5.3 oAuth - Comprehensive AuditEvent record

This pattern preserves most oAuth JWT fields in the AuditEvent. Not all are preserved as some attributes are proven during the oAuth token validation and thus carry no further information useful in an AuditEvent (e.g., not before). 

Local policy may choose to record less than are defined here, or may choose to record more. 

For those using oAuth beyond IUA, there is no specific guidance here.

- [StructureDefinition profile of Basic AuditEvent pattern for oAuth Comprehensive](StructureDefinition-IHE.BasicAudit.OAUTHaccessTokenUse.Comprehensive.html)
  - [examples](StructureDefinition-IHE.BasicAudit.OAUTHaccessTokenUse.Comprehensive-examples.html)

#### 3:5.7.5.4 oAuth mapping to AuditEvent

The following table uses a short-hand for the oAuth fields and FHIR AuditEvent elements to keep the table compact. It is presumed the reader can understand the field and the FHIR AuditEvent element given. This also presumes that the one recording the AuditEvent has this level access to the oAuth fields. Note: Prefix with a ":" is the specification that defined that field (IHE-IUA, IHE-BPPC, etc.).

Note: Opaque is not shown in the table as it has no access to any oAuth Fields.

| oAuth field                          | Comprehensive AuditEvent          | Minimal AuditEvent                |
|--------------------------------------|-----------------------------------|-----------------------------------|
| iss (JWT Issuer)                     | agent[user].who.identifier.system | agent[user].who.identifier.system |
| sub (JWT Subject)                    | agent[user].who.identifier.value  | agent[user].who.identifier.value |
| aud (JWT Audience)                   |   | |
| jti (JWT ID)                         | agent[user].policy                | agent[user].policy |
| exp (JWT Expiration Time)            |   | |
| nbf (JWT Not before)                 |   | |
| iat (JWT Issued at)                  |   | |
| client_id (OA2 client id)            | agent[client].who.identifier.value | agent[client].who.identifier.value |
| scope (OA2 token scope)              |   | |
| ihe_iua:subject_name                 | agent[user].who.display | |
| ihe_iua:subject_organization         | agent[userorg].who.display | |
| ihe_iua:subject_organization_id      | agent[userorg].who.identifier.value | |
| ihe_iua:subject_role                 | agent[user].role | agent[user].role |
| ihe_iua:purpose_of_use               | agent[user].purposeOfUse           | agent[user].purposeOfUse |
| ihe_iua:home_community_id            | entity[consent].what.identifier.assigner.identifier.value | |
| ihe_iua:national_provider_identifier | agent[user].extension[otherId][npi].identifier.value | |
| ihe_iua:person_id                    | agent[user].extension[otherId][provider-id].identifier.value | |
| ihe_bppc:patient_id                  | entity[consent-patient].what.identifier.value | |
| ihe_bppc:doc_id                      | entity[consent].what.identifier.value | |
| ihe_bppc:acp                         | entity[consent].detail.valueString | |
{:.grid}

### 3:5.7.6 Consent Authorized Decision Audit Message

This set of auditEvent messages would be recorded by an Authorization Service when an authorization decision is rendered that is based on a Consent. This AuditEvent is not specific to the authorization technology (SAML/XACML, Oauth, other). This AuditEvent is specific to describing the Consent involved in the authorization decision. Success would indicate that a Permit is allowed, Failure would indicate that Deny was returned as the authorization decision.

- [Authorization Decision inclusive of Consent](StructureDefinition-IHE.BasicAudit.AuthZconsent.html)
  - [examples](StructureDefinition-IHE.BasicAudit.AuthZconsent-examples.html)

### 3:5.7.7 Privacy Disclosure Audit Message

This is transformed from the content defined today in ATNA \[ITI-20\] [2:3.20.8 Disclosure audit message](https://profiles.ihe.net/ITI/TF/Volume2/ITI-20.html#3.20.8). It is replicated here for ease of understanding, and is presented here using the FHIR AuditEvent. There are some differences caused and enabled by the FHIR AuditEvent schema and profiling.

In some countries a Patient has the right to get an accounting of disclosures. This report includes disclosures of their data that meet regulatory criteria. Most audit events, including export events, must be post-analyzed to determine whether they describe an event that needs to be included in the accounting of disclosures. For example, in the USA these rules are defined in [HIPAA](https://www.cdc.gov/phlp/publications/topic/hipaa.html), and only a few kinds of export events meet the criteria to be included in an accounting of disclosures report. When it is known, at the time the event is recorded that the event is indeed a disclosure, the disclosure audit message can be used to document the event.

The requirements of an accounting of disclosures are defined in [ASTM-2147](https://www.astm.org/e2147-18.html). A disclosure shall include the following, when the value is known:

- Who did the disclosure (the releasing agent), 
- When did the disclosure happen,
- Who was the data disclosed to (receiving agent machine and other parties, if known),
- What patient was involved (multiple patients would be done as multiple audit entries), 
- What data was involved, and 
- Why the disclosure was done 

There is some other information that may be available:

- Who is the custodian of the data (the official organization responsible), and
- Who authorized the release such as a guardian or relative (authorizing agent)

The following is the layout of the Disclosure audit event. This pattern will be extended and modified by applications when appropriate.

- [Disclosure reported by the Recipient](StructureDefinition-IHE.BasicAudit.PrivacyDisclosure.Recipient.html)
  - [examples](StructureDefinition-IHE.BasicAudit.PrivacyDisclosure.Recipient-examples.html)
- [Disclosure reported by the Source](StructureDefinition-IHE.BasicAudit.PrivacyDisclosure.Source.html)
  - [examples](StructureDefinition-IHE.BasicAudit.PrivacyDisclosure.Source-examples.html)
  
