Profile:        AuditPrivacyDisclosureRecipient
Parent:         AuditEvent
Id:             IHE.BasicAudit.PrivacyDisclosure.Recipient
Title:          "Audit Event for a Privacy Disclosure as recorded by a Recipient"
Description:    """
Defines constraints on the AuditEvent Resource to record when a Privacy Disclosure is detected at the Recipient of the data.

- Import event
- shall have source of itself
- shall have a source agent
- shall have a recipient agent
- may have user, app, organization agent(s)
  - combine with the Security Token pattern
- may, if known, have the custodian that released the data
- may, if known, have the authorizer that represented the patient (may be the patient)
- shall have a patient entity
- shall have a set identity entity
"""
* modifierExtension 0..0
* type 1..
* type = DCM#110107 // "Import"
* action = #C
* insert requireAtLeastOneMatch(subtype, disclosure, http://terminology.hl7.org/CodeSystem/iso-21089-lifecycle#disclose )
* subtype ^slicing.description = "needs to at least be a disclosure event"
// * severity in R5
* recorded 1..1 // already required
* outcome 1..1
* outcomeDesc MS // encouraged
* purposeOfEvent MS // why was the data disclosed
// source is already required, see invariant val-audit-source use
* agent 2..*
* agent ^slicing.discriminator.type = #value
* agent ^slicing.discriminator.path = "type"
* agent ^slicing.rules = #open
* agent ^slicing.description = "source, recipient, and possibly the user who participated"
* agent contains 
	source 1..1 and
	recipient 1..1 and
	custodian 0..1 and
	authorizer 0..1
	// may be many including app identity, user identity, etc
	// likely do need to express WHO requested the data be disclosued, and WHO authorized the disclosure
* agent[source].type 1..1
* agent[source].type = DCM#110153 // "Source Role ID"
* agent[source].who 1..1
* agent[source].network 1..1
* agent[recipient] obeys val-audit-source
* agent[recipient].type 1..1
* agent[recipient].type = DCM#110152 // "Destination Role ID"
* agent[recipient].who 1..1
* agent[recipient].network 1..1
* agent[custodian].type = SCT#159541003 // "Record keeping/library clerk"
* agent[custodian].who 1..1
* agent[authorizer].type 1..1
* agent[authorizer].type = SCT#429577009 // "Patient Advocate"
* agent[authorizer].who 1..1
* entity 2..*
* entity ^slicing.discriminator.type = #value
* entity ^slicing.discriminator.path = "type"
* entity ^slicing.rules = #open
* entity ^slicing.description = "patient and some data involved"
* entity contains
	patient 1..1 
* entity[patient].type 1..1
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 // "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 // "Patient"
* entity[patient].role 1..
* entity[patient].what 1..1
* entity[patient].what only Reference(Patient)



Invariant: val-audit-source
Description: "The Audit Source is this agent too."
Expression: "$this.who = %resource.source.observer"
Severity: #error

Profile:        AuditPrivacyDisclosureSource
Parent:         AuditEvent
Id:             IHE.BasicAudit.PrivacyDisclosure.Source
Title:          "Audit Event for Privacy Disclosure at Source"
Description:    """
Defines constraints on the AuditEvent Resource to record when a Privacy Disclosure happens at the Source.

- Export event
- shall have source of itself
- shall have a source agent
- shall have a recipient agent
- may have user, app, organization agent(s)
  - combine with the Security Token pattern
- should have the custodian that released the data
- should have the authorizer that represented the patient (may be the patient)
- shall have a patient entity
- shall have the set of data entity(ies)
"""
* modifierExtension 0..0
* type = DCM#110106 // "Export"
* type 1..
* action = #R
* insert requireAtLeastOneMatch(subtype, disclosure, http://terminology.hl7.org/CodeSystem/iso-21089-lifecycle#disclose)
* subtype ^slicing.description = "needs to at least be a disclosure event"
// * severity in R5
* recorded 1..1 // already required
* outcome 1..1
* outcomeDesc MS // encouraged
* purposeOfEvent MS // why was the data disclosed
// source is already required, see invariant val-audit-source use
* agent 2..*
* agent ^slicing.discriminator.type = #value
* agent ^slicing.discriminator.path = "type"
* agent ^slicing.rules = #open
* agent ^slicing.description = "source, recipient, and possibly the user who participated"
* agent contains 
	source 1..1 and
	recipient 1..1 and
	custodian 0..1 and
	authorizer 0..1
	// may be many including app identity, user identity, etc
	// likely do need to express WHO requested the data be disclosued, and WHO authorized the disclosure
* agent[source].type 1..1
* agent[source].type = DCM#110153 // "Source Role ID"
* agent[source].who 1..1
* agent[source].network 1..1
* agent[source] obeys val-audit-source
* agent[recipient].type 1..1
* agent[recipient].type = DCM#110152 // "Destination Role ID"
* agent[recipient].who 1..1
* agent[recipient].network 1..1
* agent[custodian].type 1..1
* agent[custodian].type = SCT#159541003 // "Record keeping/library clerk"
* agent[custodian].who 1..1
* agent[authorizer].type 1..1
* agent[authorizer].type = SCT#429577009 // "Patient Advocate"
* agent[authorizer].who 1..1
* entity 2..*
* entity ^slicing.discriminator.type = #value
* entity ^slicing.discriminator.path = "type"
* entity ^slicing.rules = #open
* entity ^slicing.description = "patient and some data involved"
* entity contains
	patient 1..1
* entity[patient].type 1..1
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 // "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 // "Patient"
* entity[patient].role 1..
* entity[patient].what 1..1
* entity[patient].what only Reference(Patient)




Instance: ex-auditPrivacyDisclosure-recipient
InstanceOf: IHE.BasicAudit.PrivacyDisclosure.Recipient
Title: "Audit Example of Privacy Disclosure at recipient"
Description: "Audit Example for a Privacy Disclosure as recorded at the recipient"
Usage: #example
* meta.security = http://terminology.hl7.org/CodeSystem/v3-ActReason#HTEST
* type = DCM#110107 "Import"
* action = #C
* subtype[disclosure] = http://terminology.hl7.org/CodeSystem/iso-21089-lifecycle#disclose "Disclose Record Lifecycle Event"
//* severity = #Informational
* recorded = 2020-04-29T09:49:00.000Z
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#0 "Success"
* purposeOfEvent = http://terminology.hl7.org/CodeSystem/v3-ActReason#PATRQT "patient requested"
* source.site = "server.example.com"
* source.observer = Reference(Device/ex-device)
* source.type = http://terminology.hl7.org/CodeSystem/security-source-type#4 "Application Server"
* agent[source].type = DCM#110153 "Source Role ID"
* agent[source].requestor = false
* agent[source].who.display = "myMachine.example.org"
* agent[source].network.address = "2001:0db8:85a3:0000:0000:8a2e:0370:7334"
* agent[source].network.type = http://hl7.org/fhir/network-type#2 "IP Address"
* agent[recipient].type = DCM#110152 "Destination Role ID"
* agent[recipient].who = Reference(Device/ex-device)
* agent[recipient].requestor = false
* agent[recipient].network.address = "http://server.example.com/fhir"
* agent[recipient].network.type = http://hl7.org/fhir/network-type#5 "URI"
* agent[authorizer].type = SCT#429577009 "Patient Advocate"
* agent[authorizer].who = Reference(Patient/ex-patient)
* agent[authorizer].requestor = true
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 "Patient"
* entity[patient].what = Reference(Patient/ex-patient)
* entity[1].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#2 "System Object"
* entity[1].role = http://terminology.hl7.org/CodeSystem/object-role#3 "Report"
* entity[1].what = Reference(DocumentReference/ex-documentreference)


Instance: ex-auditPrivacyDisclosure-source
InstanceOf: IHE.BasicAudit.PrivacyDisclosure.Source
Title: "Audit Example of Privacy Disclosure at source"
Description: "Audit Example for a Privacy Disclosure from source perspective"
Usage: #example
* meta.security = http://terminology.hl7.org/CodeSystem/v3-ActReason#HTEST
* type = DCM#110106 "Export"
* action = #R
* subtype[disclosure] = http://terminology.hl7.org/CodeSystem/iso-21089-lifecycle#disclose "Disclose Record Lifecycle Event"
* subtype[1] = http://hl7.org/fhir/restful-interaction#read "read"
//* severity = #Informational
* recorded = 2020-04-29T09:49:00.000Z
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#0 "Success"
* purposeOfEvent = http://terminology.hl7.org/CodeSystem/v3-ActReason#PATRQT "patient requested"
* source.site = "mobile app foo-bar"
* source.observer.display = "model number ABC, serial number 1234"
* source.type = http://terminology.hl7.org/CodeSystem/security-source-type#1 "User Device"
* agent[source].type = DCM#110153 "Source Role ID"
* agent[source].who.display = "model number ABC, serial number 1234"
* agent[source].requestor = false
* agent[source].network.address = "myDevice.example.com"
* agent[source].network.type = http://hl7.org/fhir/network-type#1 "domain name"
* agent[recipient].type = DCM#110152 "Destination Role ID"
* agent[recipient].requestor = false
* agent[recipient].who.display = "myMachine.example.org"
* agent[recipient].network.address = "http://server.example.com/fhir"
* agent[recipient].network.type = http://hl7.org/fhir/network-type#5 "URI"
* agent[authorizer].type = SCT#429577009 "Patient Advocate"
* agent[authorizer].who = Reference(Patient/ex-patient)
* agent[authorizer].requestor = true
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 "Patient"
* entity[patient].what = Reference(Patient/ex-patient)
* entity[1].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#2 "System Object"
* entity[1].role = http://terminology.hl7.org/CodeSystem/object-role#3 "Report"
* entity[1].what = Reference(DocumentReference/ex-documentreference)

Instance: ex-auditPrivacyDisclosure-source2
InstanceOf: IHE.BasicAudit.PrivacyDisclosure.Source
Title: "Audit Example of Privacy Disclosure at source"
Description: "Audit Example for a Privacy Disclosure from source perspective"
Usage: #example
* meta.security = http://terminology.hl7.org/CodeSystem/v3-ActReason#HTEST
* type = DCM#110106 "Export"
* action = #R
* subtype[disclosure] = http://terminology.hl7.org/CodeSystem/iso-21089-lifecycle#disclose "Disclose Record Lifecycle Event"
* subtype[1] = http://hl7.org/fhir/restful-interaction#read "read"
//* severity = #Informational
* recorded = 2020-04-29T09:49:00.000Z
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#0 "Success"
* purposeOfEvent = http://terminology.hl7.org/CodeSystem/v3-ActReason#PATRQT "patient requested"
* source.site = "mobile app foo-bar"
* source.observer.display = "model number ABC, serial number 1234"
* source.type = http://terminology.hl7.org/CodeSystem/security-source-type#1 "User Device"
* agent[source].type = DCM#110153 "Source Role ID"
* agent[source].who.display = "model number ABC, serial number 1234"
* agent[source].requestor = false
* agent[source].network.address = "myDevice.example.com"
* agent[source].network.type = http://hl7.org/fhir/network-type#1 "domain name"
* agent[recipient].type = DCM#110152 "Destination Role ID"
* agent[recipient].requestor = false
* agent[recipient].who.display = "myMachine.example.org"
* agent[recipient].network.address = "http://server.example.com/fhir"
* agent[recipient].network.type = http://hl7.org/fhir/network-type#5 "URI"
* agent[authorizer].type = SCT#429577009 "Patient Advocate"
* agent[authorizer].who = Reference(Patient/ex-patient)
* agent[authorizer].requestor = true
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 "Patient"
* entity[patient].what = Reference(Patient/ex-patient)
* entity[1].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#2 "System Object"
* entity[1].role = http://terminology.hl7.org/CodeSystem/object-role#3 "Report"
* entity[1].what = Reference(DocumentReference/ex-documentreference)


Instance: ex-auditPrivacyDisclosure-measurereport
InstanceOf: IHE.BasicAudit.PrivacyDisclosure.Source
Title: "Audit Example of Privacy Disclosure of a patient specific MeasureReport"
Description: "Audit Example for a Privacy Disclosure from source perspective of a MeasureReport"
Usage: #example
* meta.security = http://terminology.hl7.org/CodeSystem/v3-ActReason#HTEST
* type = DCM#110106 "Export"
* action = #R
* subtype[disclosure] = http://terminology.hl7.org/CodeSystem/iso-21089-lifecycle#disclose "Disclose Record Lifecycle Event"
* subtype[1] = http://hl7.org/fhir/restful-interaction#read "read"
//* severity = #Informational
* recorded = 2020-04-29T09:49:00.000Z
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#0 "Success"
* purposeOfEvent = http://terminology.hl7.org/CodeSystem/v3-ActReason#PATRQT "patient requested"
* source.site = "mobile app foo-bar"
* source.observer.display = "model number ABC, serial number 1234"
* source.type = http://terminology.hl7.org/CodeSystem/security-source-type#1 "User Device"
* agent[source].type = DCM#110153 "Source Role ID"
* agent[source].who.display = "model number ABC, serial number 1234"
* agent[source].requestor = false
* agent[source].network.address = "myDevice.example.com"
* agent[source].network.type = http://hl7.org/fhir/network-type#1 "domain name"
* agent[recipient].type = DCM#110152 "Destination Role ID"
* agent[recipient].requestor = false
* agent[recipient].who.display = "myMachine.example.org"
* agent[recipient].network.address = "http://server.example.com/fhir"
* agent[recipient].network.type = http://hl7.org/fhir/network-type#5 "URI"
* agent[authorizer].type = SCT#429577009 "Patient Advocate"
* agent[authorizer].who = Reference(Patient/ex-patient)
* agent[authorizer].requestor = true
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 "Patient"
* entity[patient].what = Reference(Patient/ex-patient)
* entity[1].type = http://hl7.org/fhir/resource-types#MeasureReport
* entity[1].role = http://terminology.hl7.org/CodeSystem/object-role#3 "Report"
* entity[1].what = Reference(MeasureReport/ex-measurereport)


Instance: ex-auditPrivacyDisclosure-recipient-minCodes
InstanceOf: IHE.BasicAudit.PrivacyDisclosure.Recipient
Title: "Audit Example of Privacy Disclosure at recipient"
Description: "Audit Example for a Privacy Disclosure as recorded at the recipient"
Usage: #example
* meta.security = http://terminology.hl7.org/CodeSystem/v3-ActReason#HTEST
* type = DCM#110107 // "Import"
* action = #C
* subtype[disclosure] = http://terminology.hl7.org/CodeSystem/iso-21089-lifecycle#disclose // "Disclose Record Lifecycle Event"
//* severity = #Informational
* recorded = 2020-04-29T09:49:00.000Z
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#0 // "Success"
* purposeOfEvent = http://terminology.hl7.org/CodeSystem/v3-ActReason#PATRQT // "patient requested"
* source.site = "server.example.com"
* source.observer = Reference(Device/ex-device)
* source.type = http://terminology.hl7.org/CodeSystem/security-source-type#4 // "Application Server"
* agent[source].type = DCM#110153 // "Source Role ID"
* agent[source].requestor = false
* agent[source].who.display = "myMachine.example.org"
* agent[source].network.address = "2001:0db8:85a3:0000:0000:8a2e:0370:7334"
* agent[source].network.type = http://hl7.org/fhir/network-type#2 // "IP Address"
* agent[recipient].type = DCM#110152 "Destination Role ID"
* agent[recipient].who = Reference(Device/ex-device)
* agent[recipient].requestor = false
* agent[recipient].network.address = "http://server.example.com/fhir"
* agent[recipient].network.type = http://hl7.org/fhir/network-type#5 // "URI"
* agent[authorizer].type = SCT#429577009 "Patient Advocate"
* agent[authorizer].who = Reference(Patient/ex-patient)
* agent[authorizer].requestor = true
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 // "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 // "Patient"
* entity[patient].what = Reference(Patient/ex-patient)
* entity[1].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#2 // "System Object"
* entity[1].role = http://terminology.hl7.org/CodeSystem/object-role#3 // "Report"
* entity[1].what = Reference(DocumentReference/ex-documentreference)


