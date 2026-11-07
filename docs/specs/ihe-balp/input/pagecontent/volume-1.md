
The Basic Audit Log Patterns (BALP) is a Content Profile that defines some basic and reusable AuditEvent patterns. 

The Audit Log Patterns defined here rely on the [ATNA](https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html) Profile for transport of the AuditEvent and query/retrieval of AuditEvents previously recorded. 
The patterns defined here may be used as they are, or further refined to specific use-cases. 
Where a more specific audit event is defined, it should be derived off of these basic patterns. Thus, a more specific AuditEvent would be compliant with one or more of the AuditEvent patterns defined here.

This Implementation Guide is intended to be fully compliant with the HL7 [FHIR](http://hl7.org/fhir/) specification, providing only use-case driven constraints to aid with interoperability, deterministic results, and compatibility with [ATNA](https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html) and other IHE Profiles.

Figure 1:52-1 shows a broader setting for the actors directly involved in the Basic Audit Log Patterns Profile and the relevant transactions between them. 

<div>
{%include ActorContext.svg%}
</div>
<br clear="all">

**Figure 1:52-1: BasicAudit Actor Setting**

*ANY Secure Client* and *ANY Secure Server* represent abstractions of any client/server actor grouped with an **ATNA Secure Node** or an **ATNA Secure Application** supporting the **ATX: FHIR Feed Option**, defined in the [RESTful ATNA Supplement](https://www.ihe.net/uploadedFiles/Documents/ITI/IHE_ITI_Suppl_RESTful-ATNA.pdf). Where *ANY Secure Client* and *ANY Secure Server* are involved in some communication that is an auditable event described in this Implementation Guide and for which some AuditEvent pattern is defined. The AuditEvent patterns defined here will be created and recorded \[ITI-20\] by the *Secure Node* or *Secure Application* that is grouped within the diagramed *ANY Secure Client* and the *ANY Secure Server*. 

The double recording enables forensic analysis to detect failures better. Both audit events recorded will be different as the AuditEvent.source would identify the actor recording the event. Some actors will be able to populate the AuditEvent pattern given more fully, the lack of an element being populated is not a defect, but rather indicates that the actor did not have access to that data. 

## 1:52.1 BasicAudit Actors and Content

Figure 1:52.1-1 shows the actors directly involved in the Basic Audit Log Patterns Profile and the relevant transactions between them. The **Audit Creator** creates AuditEvent records following this profile that is consumed by the **Audit Consumer**. The transport for the AuditEvent records is the [ATNA](https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html) Profile.

<div>
{%include Actors.svg%}
</div>
<br clear="all">

**Figure 1:52.1-1: BasicAudit Actor Diagram**

**Table 1:52.1-1: BasicAudit - Actors**  

| Actors                  |  Optionality| Reference|
|-------------------------|:----------:|----------|
| Audit Creator           | R | 1:52.1.1.1 |
| Audit Consumer          | R | 1:52.1.1.2 |
{:.grid}


### 1:52.1.1 Actor Descriptions and Actor Profile Requirements


#### 1:52.1.1.1 Audit Creator

The Audit Creator shall detect the defined auditable events and record a complaint AuditEvent as defined. The Audit Creator shall be grouped with an **ATNA Secure Application** or **ATNA Secure Node** with support for **ATNA ATX:FHIR Feed Option** defined in the [RESTful ATNA Supplement](https://www.ihe.net/uploadedFiles/Documents/ITI/IHE_ITI_Suppl_RESTful-ATNA.pdf) for the recording of the compliant AuditEvent to the Audit Record Repository.

A `requirements` CapabilityStatements defines full functional [Audit Creator](CapabilityStatement-IHE.BALP.AuditCreator.html) with ATNA support for ITI-20.

#### 1:52.1.1.2 Audit Consumer

The Audit Consumer shall be grouped with an **ATNA Audit Consumer** with support for the **Retrieve Audit Message Option** defined in the [RESTful ATNA Supplement](https://www.ihe.net/uploadedFiles/Documents/ITI/IHE_ITI_Suppl_RESTful-ATNA.pdf). The Audit Consumer understands the AuditEvent profiles defined, but shall not reject an AuditEvent due to non-compliance.

A `requirements` CapabilityStatements defines full functional [Audit Consumer](CapabilityStatement-IHE.BALP.AuditConsumer.html) with ATNA support for ITI-81.

#### 1:52.1.1.3 ATNA Audit Record Repository

The [ATNA](https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html) Audit Record Repository with support for **ATNA ATX:FHIR Feed Option**, and **Retrieve Audit Message Option**. These options are defined in the [RESTful ATNA Supplement](https://www.ihe.net/uploadedFiles/Documents/ITI/IHE_ITI_Suppl_RESTful-ATNA.pdf). The ATNA Audit Record Repository may detect and validate the AuditEvent requirements defined here, but shall not reject an AuditEvent due to non-compliance.

A `requirements` CapabilityStatements defines full functional [ATNA Audit Record Repository](CapabilityStatement-IHE.BALP.ATNA.AuditRecordRepository.html) with support for the BALP content profiles.

## 1:52.2 BasicAudit Actor Options

Each AuditEvent pattern defined here can be declared by an **Audit Creator** or an **Audit Consumer**.

## 1:52.3 BasicAudit Required Actor Grouping

The required groupings for BasicAudit are those in the ATNA Profile, see [IHE ITI TF-1:9.3](https://profiles.ihe.net/ITI/TF/Volume1/ch-9.html).

The Security Considerations page describes some optional groupings that may be of interest for security considerations.

Cross-Profile Considerations describes some optional groupings in other related profiles.

## 1:52.4 BasicAudit Overview

### 1:52.4.1 Concepts
This guide provides reusable AuditEvent patterns that can be used directly, or combined, or as derivation material for a more specific AuditEvent. 

##### 1:52.4.1.1 Data access requests produce an AuditEvent

The following use-case represents a generic interaction between a client and a server. The interactions 1, 2, and 4 represent the activity that is auditable. The interaction 3 is the recording of the AuditEvent following this Implementation Guide by the server. The interaction 5 is the recording of the AuditEvent following this Implementation Guide by the client.

<div>
{%include usecase1-processflow.svg%}
</div>
<br clear="all">

**Figure: 1:52.4.1.1-1: Audit log event creation**

#### 1:52.4.1.2 AuditEvents are available to authorized clients

The following use-case represents the use-case defined in [ATNA \[ITI-81\]](https://www.ihe.net/uploadedFiles/Documents/ITI/IHE_ITI_Suppl_RESTful-ATNA.pdf). The use of this transaction in this Implementation Guide is to specialize it with the use of the patient identity to retrieve AuditEvent resources where that patient identity was found in the .agent or .entity. Where the patient identity in the .agent element indicates that the patient was actively engaged in the activity recorded in that AuditEvent, and .entity element indicates that the patient was the subject of the event. Note that accessing the audit log is an auditable event and thus interactions 3 and 5.

<div>
{%include usecase2-processflow.svg%}
</div>
<br clear="all">

**Figure: 1:52.4.1.2-1: Audit Use**

#### 1:52.4.1.3 Minimal vs Comprehensive AuditEvent

In this Implementation Guide there are sometimes two sets of patterns defined:

##### 1:52.4.1.3.1 Minimal AuditEvent

The minimal AuditEvent pattern should be used whenever possible. The minimal AuditEvent pattern avoids replicating information that can be looked up by any audit analysis application that has access. By placing minimal information into the AuditEvent the size of the AuditEvent is dramatically reduced, and there is less exposure of privacy and security information. 

With minimal AuditEvent the AuditEvent contains identifiers or References, and not the details. For example, user identifier, practitioner identifier, organizational identifier, patient identifier, data object identifier, location identifiers, and security token identifier. With minimal the users names would not be recorded, details of a diagnosis would not be replicated, postal addresses would not be replicated. The identifiers that are recorded can be looked up using a user directory, practitioner directory, patient directory, data registry, and token introspection. The lookup of these values is further protection of the data behind them, preventing abuse by someone who has access to the AuditEvent log for one purpose gaining knowledge from other AuditEvent records.

With minimal AuditEvent, the AuditEvent contains only the search (aka, query) parameters, and never the results of the search. This too limits the size of the AuditEvent and the exposure of sensitive results data into the AuditEvent log. The results can be determined by re-executing the search or query, adjusting the results based on the time of the search. 

The minimal AuditEvent is lower risk, but would still contain sensitive data. 

##### 1:52.4.1.3.2 Comprehensive AuditEvent

There are times when the AuditEvent log are used by Audit Consumer Actors are known to not have access to some lookup services. In these cases, it is better to record the details into the AuditEvent. For example, where in a Cross-Community setting the details of the user may not be possible to lookup the security token details.

With Comprehensive AuditEvent, the search results might be preserved in the AuditEvent. This will create a very large AuditEvent with replicated data. So, care should be taken to select the kinds of auditable events where the results are recorded comprehensively, and these results should be purged regularly to limit the overall impact as the results data tend to be less useful as the AuditEvent ages. 

In Cross-Community settings making full access to all of the needed directory and registry services may not be logistically possible. The counter argument is that when one party needs to perform AuditEvent log analysis and has suspicion based on that analysis, they could use out-of-band methods to request a lookup of an identifier. For example, when the AuditEvent log shows a strange behavior by a user identifier, such as an unusually large number of search activities at strange times of day or searches against a VIP patient. These out-of-band requests should be supported by the Cross-Community policy agreements.

#### 1:52.4.1.4 Query Parameter Handling

The raw search request is base64 encoded and placed in the .entity[query].query element. The base64 encoding of the raw search request enables preserving exactly what was requested, including possibly malicious patterns. This enables detection of malicious or malformed requests.

The cleaned search may be recorded (not base64) in the .entity[query].description. The cleaned search request would have removed parameters that were not understood/supported. The cleaned search request in the .description element enables more efficient processing.

The results of the search are not included in the minimal AuditEvent, and are recommended to not be included in the comprehensive AuditEvent. Recording the results of a search in the AuditEvent will produce very large resources that are hard to process, and which replicate the database searched multiple times over. The AuditEvent record with search results contain highly sensitive data.

#### 1:52.4.1.5 Best Effort

The AuditEvent should be populated with best-effort. This principle recognizes that there are times when an AuditEvent can't be fully populated due to the recording entity not having access to all of the information. The principle recognizes that recording some information is better than failing to record any AuditEvent due to this lack of elements to fit the pattern. These cases where lesser information is available to be recorded tend to be during failure-modes, which leads to the importance to record the elements that are known.

This principle also recognizes that some detail could be obtained, but through extreme effort. This extreme effort is often not justified. So best-effort does not compel the use of extreme effort to obtain the missing information. For example, during a network transaction where the AuditEvent pattern is calling for the name of the user in addition to the user identifier, the name could be looked up in a directory, but this would be extreme effort that is not called for by best-effort. 

#### 1:52.4.1.6 Patient as a Subject

It is a best practice to include a reference to the Patient/Subject affected by any auditable event, in order to enable Privacy Accounting of Disclosures and Access Logs, and to enable privacy office and security office audit log analysis. Reasonable efforts should be taken to assure the Patient/Subject is recorded, but it is recognized that there are times when this is not reasonable, see the Best Effort section above.

The Patient as the subject of a transaction may be a search parameter (e.g. `patient` or `subject`), as scoped as part of a request security context such as IUA, or where the resource has a reference to a Patient (e.g. Observation.subject, DiagnosticReport.subject).

The Patient/Subject of an activity is indicated in an .entity element; with the .entity.who indicating the Patient reference, and the .entity.type indicating “1” Person, and the .entity.role indicating “1” patient. No other elements in this .entity need to be filled out. The indicator of the .entity.who, .entity.type, and .entity.role are enough to indicate that this AuditEvent activity has a subject as indicated.

Where an activity impacts more than one Patient/Subject, multiple AuditEvent resources should be recorded, one for each Patient/Subject. This best enables segmentation of the AuditEvent details so as to limit the Privacy impact. The use of multiple AuditEvent is a best-practice and should be driven by a Policy. There will be cases where the use of multiple AuditEvent resources are not necessary, such as public health reporting.

To record a REST interaction or $operation, it is often necessary to complete the transaction in order to determine the Patient/Subject. Inspection of the potential returned results may be necessary. Some REST Search/Query requests and $operations include parameters limiting the results to a specific Patient, in these cases this parameter informs the inclusion of the Patient reference.

The Patient as a Subject is distinct from when the Patient is an Agent. When the Patient is the User, they would be indicated in an .agent and the .entity element would not be necessary. Some examples would be when the Patient is using a Patient-Portal, or when the Patient is authoring Patient Generated Health Data (PGHD).  Note that in SAML and oAuth the subject of these security tokens is the user. This is contextual use of the word 'subject', as in the context is the security token subject is the user agent.

Finding AuditEvents by Patient:
- Finding AuditEvents where the Patient is an agent, one would use the `agent` search parameter.
- Finding AuditEvents where the Patient is a subject, one would use the `entity` search parameter.
- Finding AuditEvents where the Patient is either an agent or subject, one would use the `patient` search parameter.

This Implementation Guide includes profiles that include the Patient as the subject.

### 1:52.4.2 Use Cases

The AuditEvent patterns defined here cover the following Use Cases.

#### 1:52.4.2.1 Use Case #1: Generic AuditEvent pattern for FHIR RESTful operations

Given the http RESTful operations of Create, Read, Update, Delete, and Search; the patterns defined here will capture sufficient information for audit analysis by an Audit Consumer Actor. See [3:5.7.3 RESTful activities](content.html#3573-restful-activities) for the pattern definitions and examples.

The Use Case #1 is represented by the figure first introduced in section [1:52.4.1.1 Data access requests produce an AuditEvent](#152411-data-access-requests-produce-an-auditevent).
The generic interaction between a client and a server shown with interactions 1, 2, and 4 represent the activity that is auditable. The interaction 3 is the recording of the AuditEvent following this Implementation Guide by the server. The interaction 5 is the recording of the AuditEvent following this Implementation Guide by the client.

<div>
{%include usecase1-processflow.svg%}
</div>
<br clear="all">

**Figure: 1:52.4.2.1-1: Audit log event creation**

#### 1:52.4.2.2 Use Case #2: Security token enhancement pattern 

Given that a security relevant event is being recorded, for example, Generic FHIR RESTful operations, and where a security token is known that describes the client and possibly the human; there is a need to record well formatted details about the security context. This use-case needs an audit event pattern defined, to be combined with the audit event pattern for the security relevant event (see Use Case #1), that would enhance the audit event log record with details from the OAuth or SAML token. See [3:5.7.4 SAML Security Token](content.html#3574-saml-security-token) and [3:5.7.5 OAuth Security Token](content.html#3575-oauth-security-token) for the pattern definitions and examples.
  
#### 1:52.4.2.3 Use Case #3: Consent Authorized Decision event

Given that an Authorization Service makes Authorization Decisions based on a Consent, this use-case shows how the details of the authorization decision event can be recorded. See [3:5.7.6 Consent Authorized Decision Audit Message](content.html#3576-consent-authorized-decision-audit-message).

<div>
{%include usecase4-processflow.svg%}
</div>
<br clear="all">

**Figure: 1:52.4.2.3-1: Consent Authorized Decision process flow**

This diagram focuses on the Authorization Decision, so not shown in this interaction diagram are the other AuditEvent that would be recorded by the ANY Secure Client and ANY Secure Server to record the auditable event that is the client and server interaction.

#### 1:52.4.2.4 Use Case #4: Privacy relevant disclosure event

Given that a privacy relevant disclosure event is detected, this use-case shows how the details of the event can be recorded. See [3:5.7.7 Privacy Disclosure Audit Message](content.html#3577-privacy-disclosure-audit-message).

## 1:52.5 BasicAudit Security Considerations

This profile does not define any transactions. This profile defines AuditEvent patterns to be used when a more specific AuditEvent pattern is defined. 

In some cases, this profile has defined a minimally-populated AuditEvent that is focused on recording identifiers with no descriptive or replication. The minimally-populated AuditEvent helps preserve privacy of the data, actors, and events recorded in the AuditEvent. The minimally-populated AuditEvent is expected to be used by an audit log using agent that has knowledge of the source material for these identifiers, and has access rights to use those source materials. For example, recording a Practitioner identifier, without recording the Practitioner name or other elements that can be found in the Practitioner resource; where audit log analysis would use an organization directory where that identifier can be looked up to find the full details of the Practitioner.

## 1:52.6 BasicAudit Cross-Profile Considerations

This profile defines some AuditEvent patterns for use when there is not a more specific AuditEvent pattern defined. Thus, when grouped with an IHE Profile that has defined an AuditEvent (AuditMessage) pattern, that more specific pattern defined in the grouped IHE Profile shall be used.

The patterns defined here might be used as profiling parent to a more specific defined AuditEvent.
