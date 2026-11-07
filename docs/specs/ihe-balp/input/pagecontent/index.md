The **Basic Audit Log Patterns (BALP) Implementation Guide** is a **Content Profile** that defines some basic and reusable AuditEvent patterns. This includes basic audit log profiles for FHIR RESTful operations to be used when there is not a more specific audit event defined. A focus is on enabling Privacy centric AuditEvent logs that hold well-formed indication of the Patient when they are the subject of the activity being recorded in the log. Where a more specific audit event can be defined, it should be derived off of these basic patterns.

<div markdown="1" class="stu-note">

| [Significant Changes, Open and Closed Issues](issues.html) |
{: .grid}

**[Search this IG](https://www.google.com/search?q=site%3Ahttps%3A%2F%2Fprofiles.ihe.net%2FITI%2FBasicAudit)**

</div>

### Organization of This Guide

This guide is organized into following main sections:

- Volume 1: Profiles
  - [BasicAudit Introduction](volume-1.html)
  - [BasicAudit Actors and Content](volume-1.html#1521-basicaudit-actors-and-content)
  - [BasicAudit Actor Options](volume-1.html#1522-basicaudit-actor-options)
  - [BasicAudit Required Groupings](volume-1.html#1523-basicaudit-required-actor-grouping)
  - [BasicAudit Overview](volume-1.html#1524-basicaudit-overview)
  - [BasicAudit Security Considerations](volume-1.html#1525-basicaudit-security-considerations)
  - [BasicAudit Cross-Profile Considerations](volume-1.html#1526-basicaudit-cross-profile-considerations)
- Volume 3: Content Section
  - [Basic Audit Log Patterns](content.html)
    - [Scope](content.html#3571-scope)
    - [Content](content.html#3572-content)
    - [RESTful activities](content.html#3573-restful-activities)
    - [SAML Security Token](content.html#3574-saml-security-token)
    - [OAuth Security Token](content.html#3575-oauth-security-token)
    - [Consent Authorized Decision Audit Message](content.html#3576-consent-authorized-decision-audit-message)
    - [Privacy Disclosure Audit Message](content.html#3577-privacy-disclosure-audit-message)
- [Artifacts](artifacts.html)
- Other:
  - [Changes to Other IHE Specifications](other.html)
  - [Download and Analysis](download.html)
  - [AuditEvent Use in IHE Profiles](appendix.html)
  - [Test Plan](testplan.html)

See also the [Table of Contents](toc.html) and the index of [Artifacts](artifacts.html) defined as part of this implementation guide.

### Conformance Expectations

IHE uses the normative words Shall, Should, and May according to [standards conventions](https://profiles.ihe.net/GeneralIntro/ch-E.html).

#### Must Support

The use of ```mustSupport``` in StructureDefinition profiles equivalent to the IHE use of **R2** as defined in [Appendix Z](https://profiles.ihe.net/ITI/TF/Volume2/ch-Z.html#z.10-profiling-conventions-for-constraints-on-fhir).

mustSupport of true - only has a meaning on items that are minimal cardinality of zero (0), and applies only to the source actor populating the data. The source actor shall populate the elements marked with MustSupport, if the concept is supported by the actor, a value exists, and security and consent rules permit. 
The consuming actors should handle these elements being populated or being absent/empty. 
Note that sometimes mustSupport will appear on elements with a minimal cardinality greater than zero (0), this is due to inheritance from a less constrained profile.
