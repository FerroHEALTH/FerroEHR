
CodeSystem:  AuthZsubType 
Title: "Authorization subType events"
Description:  "These AuditEvent subTypes are related to Authorization Decisions. These are more specific types of Security Alert."
* ^caseSensitive = true
* ^experimental = false
* #AuthZ-Consent "Authorization Decision using Roles and Consent"
* #AuthZ-Role "Authorization Decision using only Roles"

ValueSet: AuthZsubTypeVS
Title: "Authorization subType events valueset"
Description: "ValueSet of the Authorization AuditEvent types"
* ^experimental = false
* codes from system AuthZsubType



Profile:        AuthZconsent
Parent:         AuditEvent
Id:             IHE.BasicAudit.AuthZconsent
Title:          "Basic AuditEvent pattern for when an Authorization permit is decided"
Description:    """
An AduitEvent recording a permit authorization decision by a Consent Decision Service, 

- Given an Authorization Decision resulted in a permit
- And based on a Consent resource (C1) 
- And filed by a patient (P1), 
- And in response to a request by an organization (Org1) 
- And for the purpose of treatment (TREAT).
- And the given request is authorized 
- When an AuditEvent is recorded for the activity
- Then that AuditEvent would follow this profile regarding recording the authorization decision
  - Security Alert
  - Authorization Decison by Consent
  - Execute action
  - date/time recorded
  - outcome
    - success when Permit
	- failure when Deny
	- outcomeDesc would explain why a deny
  - recorded by the authorization server
  - Agents
    - client app
	- user
	  - user requested purposeOfUse
	- user organization
	- authorization service
  - Entity
    - patient subject
	- consent on file for that patient
	- the token id (JWT ID) issued (if one is issued) should be recorded
	- other data may be recorded that was used in the decision
"""
* modifierExtension 0..0
* type = DCM#110113 // "Security Alert"
* type 1..
* subtype from AuthZsubTypeVS
* subtype 1..
* action = #E
// subtype
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
* agent ^slicing.description = "client, user, and organization requesting authorization; and authoizer"
* agent contains 
	client 1..1 and
	user 1..1 and
	userorg 1..1 and
	authorizer 1..1
* agent[client].type 1..1
* agent[client].type = http://dicom.nema.org/resources/ontology/DCM#110150 // "Application"
* agent[client].who 1..1 // client identifier, May be an Device Resource, but more likely an identifier given the App identified in the OAuth token 
* agent[client].network 1..1 // as known by TCP connection information
* agent[client].role 0..0 
* agent[client].altId 0..0
* agent[client].name 0..0 
* agent[client].location 0..0 
* agent[client].policy MS // input client token id
* agent[client].media 0..0 
* agent[client].purposeOfUse 0..0 
* agent[user].type 1..1
* agent[user].type = http://terminology.hl7.org/CodeSystem/v3-ParticipationType#IRCP // "information recipient"
* agent[user].who 1..1 // May be a Resource, but likely just an identifier from the OAuth token
* agent[user].requestor = true
* agent[user].role MS // if the OAuth token includes any roles, they are recorded here
* agent[user].altId 0..0 // discouraged
* agent[user].name MS // might also be in .who.text but here is searchable
* agent[user].location 0..0 // discouraged as unlikely to be known in this scenario
* agent[user].policy MS // input user token id
* agent[user].media 0..0 // media is physical storage media identification
* agent[user].network 0..0 // users are not network devices
* agent[user].purposeOfUse MS // if the requested purposeOfUse is applied to just the user
* agent[userorg].type 1..1
* agent[userorg].type = http://terminology.hl7.org/CodeSystem/v3-RoleClass#PROV // "healthcare provider"
* agent[userorg].who 1..1 MS
* agent[userorg].requestor = false
* agent[userorg].role 0..0
* agent[userorg].altId 0..0 // discouraged
* agent[userorg].name 0..0 
* agent[userorg].location 0..0 // discouraged as unlikely to be known in this scenario
* agent[userorg].policy 0..0
* agent[userorg].media 0..0 // media is physical storage media identification
* agent[userorg].network 0..0 // users are not network devices
* agent[userorg].purposeOfUse MS // if the request purposeOfUse is applied to the whole organization
* agent[authorizer].type 1..1
* agent[authorizer].type = http://terminology.hl7.org/CodeSystem/extra-security-role-type#authserver // "authorization server"
* agent[authorizer] obeys val-audit-source
* agent[authorizer].who 1..1
* agent[authorizer].requestor = false
* agent[authorizer].role 0..0
* agent[authorizer].altId 0..0 // discouraged
* agent[authorizer].name 0..0 
* agent[authorizer].location 0..0 // discouraged as unlikely to be known in this scenario
* agent[authorizer].policy 0..0
* agent[authorizer].media 0..0 // media is physical storage media identification
* agent[authorizer].network 0..0 // users are not network devices
* agent[authorizer].purposeOfUse 0..0
* entity 2..*
* entity ^slicing.discriminator.type = #value
* entity ^slicing.discriminator.path = "type"
* entity ^slicing.rules = #closed
* entity ^slicing.description = "patient and submission set involved"
* entity contains
	patient 1..1 and
	consent 1..* and
	token 0..1
* entity[patient].type 1..1
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 // "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 // "Patient"
* entity[patient].role 1..
* entity[patient].what 1..1
* entity[patient].what only Reference(Patient)
* entity[consent].type = http://hl7.org/fhir/resource-types#Consent // "Consent"
* entity[consent].what 1..1 MS // consent identifier
* entity[token].type 1..1
* entity[token].type = UserAgentTypes#UserOauthAgent
* entity[token].what 1..1
* entity[token].what.identifier 1..1
* entity[token].what.identifier.value 1..1
* entity[token].what.identifier.value ^short =  "jti (JWT ID)"
* entity[token].what.identifier.value ^comment =  "jti SHALL be prefixed with \"urn:ietf:params:oauth:jti:\". This URN is based on RFC3553."




////////////////////////////////////////////////////////EXAMPLES/////////////////////////////////////////////////


Instance: ex-auditAuthZconsent
InstanceOf: IHE.BasicAudit.AuthZconsent
Title: "Audit Example of a basic Authorization Permit access"
Description: """
Example AuditEvent showing an authorization decision.
"""
* meta.security = http://terminology.hl7.org/CodeSystem/v3-ActReason#HTEST
* type = DCM#110113 "Security Alert"
* subtype = AuthZsubType#AuthZ-Consent
* action = #E
//* severity = #Informational
* recorded = 2021-12-27T09:49:00.000Z
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#0 // "Success"
* source.site = "server.example.com"
* source.observer = Reference(Device/ex-authz)
* source.type = http://terminology.hl7.org/CodeSystem/security-source-type#6 // "Security Server"
* agent[authorizer].type = http://terminology.hl7.org/CodeSystem/extra-security-role-type#authserver // "authorization server"
* agent[authorizer].who = Reference(Device/ex-authz)
* agent[authorizer].requestor = false
* agent[client].type = http://dicom.nema.org/resources/ontology/DCM#110150 // "Application"
* agent[client].who = Reference(Device/ex-device)
* agent[client].network.address = "2001:0db8:85a3:0000:0000:8a2e:0370:7334"
* agent[client].network.type = http://hl7.org/fhir/network-type#2 // "IP Address"
* agent[client].requestor = false
* agent[user].type = http://terminology.hl7.org/CodeSystem/v3-ParticipationType#IRCP // "information recipient"
* agent[user].who = Reference(Practitioner/ex-practitioner)
* agent[user].purposeOfUse = http://terminology.hl7.org/CodeSystem/v3-ActReason#PATRQT
* agent[user].requestor = true
* agent[userorg].type = http://terminology.hl7.org/CodeSystem/v3-RoleClass#PROV // "healthcare provider"
* agent[userorg].who = Reference(Organization/ex-organization)
* agent[userorg].requestor = false
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 // "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 // "Patient"
* entity[patient].what = Reference(Patient/ex-patient)
* entity[consent].type = http://hl7.org/fhir/resource-types#Consent // "Consent"
* entity[consent].what = Reference(Consent/ex-consent)


Instance: ex-auditAuthZconsent-deny
InstanceOf: IHE.BasicAudit.AuthZconsent
Title: "Audit Example of a basic Authorization Deny access"
Description: """
Example AuditEvent showing an authorization decision resulting in deny.
"""
* meta.security = http://terminology.hl7.org/CodeSystem/v3-ActReason#HTEST
* type = DCM#110113 "Security Alert"
* subtype = AuthZsubType#AuthZ-Consent
* action = #E
* recorded = 2021-12-27T09:49:00.000Z
* outcome = http://terminology.hl7.org/CodeSystem/audit-event-outcome#8 "Serious failure"
* outcomeDesc = "Consent denied access to this user"
* source.site = "server.example.com"
* source.observer = Reference(Device/ex-authz)
* source.type = http://terminology.hl7.org/CodeSystem/security-source-type#6 "Security Server"
* agent[authorizer].type = http://terminology.hl7.org/CodeSystem/extra-security-role-type#authserver // "authorization server"
* agent[authorizer].who = Reference(Device/ex-authz)
* agent[authorizer].requestor = false
* agent[client].type = http://dicom.nema.org/resources/ontology/DCM#110150 // "Application"
* agent[client].who = Reference(Device/ex-device)
* agent[client].network.address = "2001:0db8:85a3:0000:0000:8a2e:0370:7334"
* agent[client].network.type = http://hl7.org/fhir/network-type#2 "IP Address"
* agent[client].requestor = false
* agent[user].type = http://terminology.hl7.org/CodeSystem/v3-ParticipationType#IRCP // "information recipient"
* agent[user].who = Reference(Practitioner/ex-practitioner)
* agent[user].purposeOfUse = http://terminology.hl7.org/CodeSystem/v3-ActReason#PATRQT
* agent[user].requestor = true
* agent[userorg].type = http://terminology.hl7.org/CodeSystem/v3-RoleClass#PROV // "healthcare provider"
* agent[userorg].who = Reference(Organization/ex-organization)
* agent[userorg].requestor = false
* entity[patient].type = http://terminology.hl7.org/CodeSystem/audit-entity-type#1 // "Person"
* entity[patient].role = http://terminology.hl7.org/CodeSystem/object-role#1 "Patient"
* entity[patient].what = Reference(Patient/ex-patient)
* entity[consent].type = http://hl7.org/fhir/resource-types#Consent // "Consent"
* entity[consent].what = Reference(Consent/ex-consent)
