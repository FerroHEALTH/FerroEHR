// This FSH file contains vocabulary unique to BasicAudit, that is used by many profiles. 
// Some vocabulary are specific to one profile, and is thus defiend with that profile.

CodeSystem: BasicAuditEntityType
Title: "Entity Types that are defined in IHE BasicAudit"
Description: """
These are new codes used in BasicAudit IG, where AuditEvent.entity is used to hold a specific kind of data that is not covered by the existing valueSet.
"""
* ^caseSensitive = true
* ^experimental = false
* #XrequestId "transport specific unique identifier where http X-Request-Id is used"



ValueSet: BasicAuditEntityTypesVS
Title: "Entity Types used by IHE BasicAudit"
Description: """
For use with AuditEvent.entity.type.  This includes codes defined in the BasicAudit.
"""
* ^experimental = false
* codes from system BasicAuditEntityType

