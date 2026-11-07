
Profile:        Create
Parent:         AuditEvent
Id:             IHE.BasicAudit.Create
Title:          "Basic AuditEvent for a successful Create not related to a Patient"
Description:    """
A basic AuditEvent profile for when a RESTful Create action happens successfully.

- Given a Resource Create is requested 
- And that resource does not have a Patient subject or is otherwise associated with a Patient
  - when the resource is Patient specific then [PatientCreate](StructureDefinition-IHE.BasicAudit.PatientCreate.html) is used
- And the request is authorized
  - Authorization failures should follow [FHIR core Access Denied](http://hl7.org/fhir/security.html#AccessDenied)
- When successful
  - Note a failure AuditEvent may follow this pattern, but would not be a successful outcome and should have an OperationOutcome
- Then the AuditEvent recorded will conform
"""
* type = http://terminology.hl7.org/CodeSystem/audit-event-type#rest // "Restful Operation"
* subtype ^slicing.discriminator.type = #value
* subtype ^slicing.discriminator.path = "$this"
* subtype ^slicing.rules = #open // allow other codes
* subtype 1..
* subtype contains anyCreate 1..1 
* subtype[anyCreate] = http://hl7.org/fhir/restful-interaction#create // "create"
* action = #C
* recorded 1..1
// failures are recorded differently
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#0 // "Success"
* outcome 1..
* agent ^slicing.discriminator.type = #value
* agent ^slicing.discriminator.path = "type"
* agent ^slicing.rules = #open
* agent 2..
* agent contains 
    client 1..1 and 
    server 1..1 and 
    user 0..1
* agent[client].type = DCM#110153 // "Source Role ID"
* agent[client].type 1..1
* agent[client].who 1..1 // client identifier, May be an Device Resource, but more likely an identifier given the App identified in the OAuth token 
* agent[client].network 1..1 // as known by TCP connection information
* agent[client].media 0..0 
* agent[server].type = DCM#110152 // "Destination Role ID"
* agent[server].type 1..1
* agent[server].who 1..1 // server identifier. May be a Device Resource, but likely just an identifier of the domain name
* agent[server].network 1..1 // as known by TCP connection information
* agent[server].media 0..0 
* agent[user].type from DataSources (required)
* agent[user].type 1..1
* agent[user].who 1..1 // May be a Resource, but likely just an identifier from the OAuth token
* agent[user].requestor = true
* agent[user].role MS // if the OAuth token includes any roles, they are recorded here
* agent[user].name MS // might also be in .who.text but here is searchable
* agent[user].media 0..0 // media is physical storage media identification
* agent[user].network 0..0 // users are not network devices
* agent[user].purposeOfUse MS // if the OAuth token includes a PurposeOfUse it is recorded here
* source MS // what agent recorded the event. Likely the client or server but might be an intermediary
* entity ^slicing.discriminator.type = #value
* entity ^slicing.discriminator.path = "type"
* entity ^slicing.rules = #open
* entity 1..
* entity contains 
	transaction 0..1 and
    data 1..1
* entity[transaction].type = BasicAuditEntityType#XrequestId
* entity[transaction].type 1..1
* entity[transaction].what.identifier.value 1..1
* entity[transaction].what.identifier.value ^short = "the value of X-Request-Id"
* entity[data].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#2 // "System Object"
* entity[data].type 1..1
* entity[data].role from RestObjectRoles (required)
* entity[data].role 1..
* entity[data].what 1..1
* entity[data].securityLabel ^short = "may contain the security labels on the resource returned"


Profile:        PatientCreate
Parent:         Create
Id:             IHE.BasicAudit.PatientCreate
Title:          "Basic AuditEvent for a successful Create with known Patient subject"
Description:    """
A basic AuditEvent profile for when a RESTful Create action happens successfully, and where there is an identifiable Patient subject associated with the create of the Resource.

- Given a Resource Create is requested 
- And that resource has a  [Patient subject](volume-1.html#152416-patient-as-a-subject) or is otherwise associated with a Patient
- And the request is authorized
  - Authorization failures should follow [FHIR core Access Denied](http://hl7.org/fhir/security.html#AccessDenied)
- When successful
  - Note a failure AuditEvent may follow this pattern, but would not be a successful outcome and should have an OperationOutcome
- Then the AuditEvent recorded will conform
"""
//* entity ^slicing.discriminator.type = #value
//* entity ^slicing.discriminator.path = "type"
//* entity ^slicing.rules = #open
* entity 2..
* entity contains 
    patient 1..1 
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 // "Person"
* entity[patient].type 1..1
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 // "Patient"
* entity[patient].role 1..
* entity[patient].what 1..1
* entity[patient].what only Reference(Patient)




ValueSet: DataSources
Title: "participant source types for RESTful create"
Description: "create agent participant types for user operators that are in REST"
* ^experimental = false
* http://terminology.hl7.org/CodeSystem/v3-ParticipationType#AUT "author (originator)" // Oct 2025 - seems to have changed from Author as validator complains
* http://terminology.hl7.org/CodeSystem/v3-ParticipationType#INF "Informant"
* http://terminology.hl7.org/CodeSystem/v3-ParticipationType#CST "Custodian"

