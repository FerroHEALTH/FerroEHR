ValueSet: AllReadVS
Title: "all Reads"
Description: "ValueSet of the restful-interaction reads"
* ^experimental = false
* http://hl7.org/fhir/restful-interaction#read // "read"
* http://hl7.org/fhir/restful-interaction#vread // "vread"

Profile:        Read
Parent:         AuditEvent
Id:             IHE.BasicAudit.Read
Title:          "Basic AuditEvent for a successful Read"
Description:    """
A basic AuditEvent profile for when a RESTful Read action happens successfully.

- Given a Resource Read is requested 
- And that resource does not have a Patient subject or is otherwise associated with a Patient
  - when the resource is Patient specific then [PatientRead](StructureDefinition-IHE.BasicAudit.PatientRead.html) is used
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
* subtype contains anyRead 1..1
* subtype[anyRead] from AllReadVS (required)
* action = #R
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
* agent[client].type = http://dicom.nema.org/resources/ontology/DCM#110152 // "Destination Role ID"
* agent[client].type 1..1
* agent[client].who 1..1 // client identifier, May be an Device Resource, but more likely an identifier given the App identified in the OAuth token 
* agent[client].network 1..1 // as known by TCP connection information
* agent[client].media 0..0 
* agent[server].type = http://dicom.nema.org/resources/ontology/DCM#110153 // "Source Role ID"
* agent[server].type 1..1
* agent[server].who 1..1 // server identifier. May be a Device Resource, but likely just an identifier of the domain name
* agent[server].network 1..1 // as known by TCP connection information
* agent[server].media 0..0 
* agent[user].type = http://terminology.hl7.org/CodeSystem/v3-ParticipationType#IRCP // "information recipient"
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
* entity[data].what 1..1
* entity[data].securityLabel ^short = "may contain the security labels on the resource returned"






Profile:        PatientRead
Parent:         Read
Id:             IHE.BasicAudit.PatientRead
Title:          "Basic AuditEvent for a successful Read with a Patient"
Description:    """
A basic AuditEvent profile for when a RESTful Read action happens successfully, and where there is an identifiable Patient subject associated with the read Resource.

- Given a Resource Read is requested 
- And that resource has a [Patient subject](volume-1.html#152416-patient-as-a-subject) or is otherwise associated with a Patient
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
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 // "Patient"
* entity[patient].role 1..
* entity[patient].what 1..1
* entity[patient].what only Reference(Patient)




ValueSet: RestObjectRoles
Title: "RESTful objects role in the event"
Description: "The role that the given Object played in the Audit Event recorded"
// I expect "Domain Resource" most of the time, but the other two are acceptable
* ^experimental = false
* http://terminology.hl7.org/CodeSystem/object-role#4 "Domain Resource"
* http://terminology.hl7.org/CodeSystem/object-role#3 "Report"
* http://terminology.hl7.org/CodeSystem/object-role#20 "Job"

