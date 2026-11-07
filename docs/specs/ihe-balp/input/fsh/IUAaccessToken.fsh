
Profile: OAUTHaccessTokenUseOpaque
Parent: AuditEvent
Id: IHE.BasicAudit.OAUTHaccessTokenUse.Opaque
Title: "Basic AuditEvent pattern for oAuth Opaque"
Description: """
Used when:
- only have an opaque oAuth token (e.g. clients).
- have access to the oAuth token, but want to log minimal details.

- oUser slice holds fragment of the opaque oAuth token
  - record only the last 32 characters of the oAuth token to limit risk or replay
  - presume 32 characters is enough to coorelate AuditEvent log entries
"""
* agent ^slicing.discriminator.type = #value
* agent ^slicing.discriminator.path = "type"
* agent ^slicing.rules = #open
* agent contains 
    oUser 1..
* agent[oUser].type 1..1
* agent[oUser].type = UserAgentTypes#UserOauthAgent
* agent[oUser] ^short = "other elements may be filled in as needed."
* agent[oUser].requestor = true
* agent[oUser].policy 1..1 MS
* agent[oUser].policy ^short = "last 32 characters of the oAuth token."
* agent[oUser].policy ^comment = "do NOT include the whole opaque token. The last 32 characters should be enough and would be the most entropy."
* agent[oUser].purposeOfUse MS 
* agent[oUser].purposeOfUse ^short = "SAML subject:purposeofuse"

Profile: OAUTHaccessTokenUseMinimal
Parent: AuditEvent
Id: IHE.BasicAudit.OAUTHaccessTokenUse.Minimal
Title: "Basic AuditEvent pattern for oAuth Opaque"
Description: """
Used when access to the oAuth token, but want to log minimal details.

- oUser slice holds only the JWT ID
"""
* agent ^slicing.discriminator.type = #value
* agent ^slicing.discriminator.path = "type"
* agent ^slicing.rules = #open
* agent contains 
    oUser 1..
* agent[oUser].type 1..1
* agent[oUser].type = UserAgentTypes#UserOauthAgent
* agent[oUser] ^short = "other elements may be filled in as needed."
* agent[oUser].requestor = true
* agent[oUser].policy 1..1 
* agent[oUser].policy ^short = "jti (JWT ID)"
* agent[oUser].policy ^comment =  "jti SHALL be prefixed with \"urn:ietf:params:oauth:jti:\". This URN is based on RFC3553."
* agent[oUser].media 0..0 // media is physical storage media identification
* agent[oUser].network 0..0 // users are not network devices

Profile: OAUTHaccessTokenUseComprehensive
Parent: AuditEvent
Id: IHE.BasicAudit.OAUTHaccessTokenUse.Comprehensive
Title:          "Basic AuditEvent pattern for when an activity was authorized by an IUA access token"
Description:    """
A basic AuditEvent profile for when an activity was authorized by an IUA access token. This profile is expected to be used with some other detail that explains the activity. This profile only covers the IUA access token.

- Given an activity has occured
- And OAuth is used to authorize (both app and user)
- And the given activity is using http with authorization: bearer mechanism 
  - IUA - [3.72 Incorporate Access Token \[ITI-72\]](https://profiles.ihe.net/ITI/IUA/index.html#372-incorporate-access-token-iti-72)
  - Bulk Data Access - [11. Presenting an Access Token to FHIR API](https://hl7.org/fhir/uv/bulkdata/authorization/index.html#presenting-an-access-token-to-fhir-api)
  - SMART-app-launch - [7.1.5 Step 4: App accesses clinical data via FHIR API](http://hl7.org/fhir/smart-app-launch/index.html#step-4-app-accesses-clinical-data-via-fhir-api)
  - [HL7 Security for Scalable Registration, Authentication, and Authorization (aka UDAP) ](http://hl7.org/fhir/us/udap-security/history.html) when it gets published 
- When an AuditEvent is recorded for the activity
- Then that AuditEvent would follow this profile regarding recording the IUA access token details
- note: this profile records minimal information from the IUA access token, which presumes that use of the AuditEvent at a later time will be able to resolve the given information.
- client slice holds the application details
  - This is likely replicated in other slices, but is consistently identified as the Application slice for ease of tracking all events caused by this client
  - place the client_id into .who.identifier.value (system is not needed, but avaialble if you have a system)
  - any network identification detail should be placed in .network (may be a IP address, or hostname)
- oUser slice holds the user details
  - user id is recorded in the .who.identifier
  - user id is also recorded in .name to be more easy searched
  - if roles or purposeOfUse are known record them here
  - the JWT ID is recorded in .policy. Expecting that during audit anaysis this ID can be looked up and dereferenced
"""
* agent ^slicing.discriminator.type = #value
* agent ^slicing.discriminator.path = "type"
* agent ^slicing.rules = #open
//* agent 1..2
* agent contains 
    oClient 1..1 and 
    oUser 0..1
* agent[oClient].type 1..1
* agent[oClient].type = http://dicom.nema.org/resources/ontology/DCM#110150 // "Application"
* agent[oClient].who 1..1 
* agent[oClient].who ^short = "May be a Resource, but likely just an identifier from the OAuth token"
* agent[oClient].who.identifier 1..1
* agent[oClient].who.identifier.value 1..1
* agent[oClient].who.identifier.value ^short = "Token client ID (client_id)"
* agent[oClient].who ^short = "client identifier"
* agent[oClient].who ^comment = "May be an Device Resource, but more likely an identifier given the App identified in the OAuth token"
* agent[oClient].network 0..1 MS 
* agent[oClient].network ^short = "The client as known by TCP connection information"
* agent[oClient].media 0..0 
* agent[oUser].type 1..1
* agent[oUser].type = http://terminology.hl7.org/CodeSystem/v3-ParticipationType#IRCP // "information recipient"
* agent[oUser].who 1..1 
* agent[oUser].who ^short = "May be a Resource, but likely just an identifier from the OAuth token"
* agent[oUser].who.display MS
* agent[oUser].who.display ^short = "User Name (USER_NAME)"
* agent[oUser].who.identifier 1..1
* agent[oUser].who.identifier.system MS
* agent[oUser].who.identifier.system ^short = "Token Issuer (TOKEN_ISSUER)"
* agent[oUser].who.identifier.value MS
* agent[oUser].who.identifier.value ^short = "User ID (USER_ID)"
* agent[oUser].requestor = true
* agent[oUser].role MS 
* agent[oUser].role ^comment = "if the OAuth token includes any roles, they are recorded here"
* agent[oUser].name MS 
* agent[oUser].name ^short = "User Name (USER_NAME)"
* agent[oUser].name ^comment = "This is more searchable than .who.display"
* agent[oUser].policy 1..1 
* agent[oUser].policy ^short = "jti (JWT ID)"
* agent[oUser].media 0..0 // media is physical storage media identification
* agent[oUser].network 0..0 // users are not network devices
* agent[oUser].purposeOfUse MS 
* agent[oUser].purposeOfUse ^comment = "if the OAuth token includes a PurposeOfUse it is recorded here"
