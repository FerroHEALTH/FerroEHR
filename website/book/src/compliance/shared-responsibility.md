# Shared responsibility

Which obligation under EU and national health-data law the software can
carry, and which stays with the organisation that deploys it, is drawn once
for the EHR system FerroEHR and FerroBRIDGE form together. The tables, one
per law (the GDPR, the EHDS, the CRA, and the national sections for the
Netherlands, Germany and Switzerland), and the account of when Cadasto B.V. is
also your processor, are published in the FerroHEALTH book:
[The EHDS EHR system: shared responsibility](https://ferrohealth.eu/docs/ehds/shared-responsibility.html).

This page keeps what is particular to a FerroEHR deployment.

## What a FerroEHR deployment sends out by default

The one thing a FerroEHR deployment sends out by default is the
[usage report](../usage-report.md): a random instance id, the version, the
licence grant type and coarse performance figures, with no patient data.
Cadasto B.V. receives it as controller for its own purposes, and its
collector also stores the IP address each report comes from. The usage report
page says how to turn it off.

## EHDS Chapter II, right by right

[EHDS Annex II point 1.3](https://eur-lex.europa.eu/eli/reg/2025/327/oj)
asks that an EHR system "be designed and developed in such a way that its
interoperability, safety and security features uphold the rights of natural
persons, in line with the intended purpose of the EHR system, as set out in
Chapter II". A person exercises most of those rights through the electronic
health data access services each Member State establishes (Art. 4(1)) and the
health professional access services (Art. 12). FerroEHR is the repository
those services read and write, so for each provision the table names three
things: the part FerroEHR provides, with the route or setting that provides
it; what you, the deploying organisation, or the Member State serve or
decide; and what a clinical data repository does not do, with the reason.

A row says "shipped" where the tracker issue that delivered the part is
closed and "planned" where it is open. Nothing in the table says that a
deployment upholds a right: that depends on the access services around the
repository and on how you configure it.

| Provision | What FerroEHR provides | What you or the Member State serve or decide | What a CDR does not do, and why |
|---|---|---|---|
| **EHDS [Art. 3(1)](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, access to one's own priority-category data "immediately after the personal electronic health data have been registered", free of charge, readable and consolidated | A committed version is readable at once over the openEHR REST API and AQL; nothing stands between the commit and the read but the authorization checks. A token with a SMART `patient/` scope or the ABAC patient claim is [held to one EHR](../smart-app-launch.md#launch-context-binding-to-one-ehr). The [`[audit.categories]`](../installation/config-audit.md#auditcategories) map tells which templates hold which priority category | Run, or connect the repository to, the Member State's access service: identify and authenticate the person, issue their token with `patient/` scopes, and fill the category map for your templates | The readable, consolidated presentation and the "free of charge" are the access service's (Art. 4(1)). A CDR serves structured data to callers and has no person-facing interface |
| **EHDS [Art. 3(2)](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, a downloadable electronic copy in the European electronic health record exchange format | The whole record as an openEHR EXTRACT, `GET /ferroehr/rest/extensions/v1/message/export/{ehr_id}`, or a selection with `POST …/message/export` ([exporting an EHR](../beyond-core/messaging.md#exporting-an-ehr)) | Offer the download in the access service, and run the conversion to the exchange format | FerroEHR writes openEHR, never the exchange format: the European interoperability software component of the FerroHEALTH EHR system is FerroBRIDGE's, and the format's specifications come from the implementing acts under Art. 15(1) |
| **EHDS [Art. 3(3)](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, a Member State's delay of the person's access "until a health professional is able to properly communicate and explain" it | The [delayed access](../security.md#delayed-access-by-the-person-ehds-art-33) under [`[access_delay]`](../installation/config-auth.md#access_delay), off by default: a new composition in a listed category is held from any request that may be the person's for at most `access_delay.hours`, the treating professional releases it with `POST /ferroehr/rest/extensions/v1/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/access-delay/release`, and every hold ends by itself when its period passes. The person's subject-scoped audit read leaves out the records about a held composition until then. Shipped, [#4351](https://github.com/FerroHEALTH/FerroEHR/issues/4351) | Turn the section on only where your Member State's law provides the delay, list the categories and the period that law sets, and have the professional release the data once explained. The hold does not filter the [audit feeds](../audit.md#sinks) or the change-event stream, so a notification you build for the person on either must hold back what is held | Which information "can have a significant impact" on a person, and how long a delay may last, is the Member State's rule under GDPR Art. 23; the conversation that explains the information is the professional's |
| **EHDS [Art. 4(2)](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, access by a person the natural person authorises, or by a legal representative, through a proxy service | A token in a patient context reaches that one EHR whoever holds it; a COMPOSITION a representative inserts names them as composer (`PARTY_RELATED`, see the Art. 5 row); every access record names the authenticated caller and the patient ([the logging elements](../audit.md#the-ehds-logging-elements-mapped)) | Run the proxy service, or connect to the Member State's: check the authorisation, its scope and its duration before you issue a token in the person's context | The register of who may act for whom, and the rules on guardians, are the Member State's proxy service (Art. 4(2) to (4)); the repository sees a token, never the authorisation behind it |
| **EHDS [Art. 5](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, inserting information in one's own EHR, "clearly distinguishable", without being able to "directly alter" what health professionals inserted | [Information a person inserts](../security.md#information-a-person-inserts-ehds-art-5): every version a `patient/`-only token commits records `PARTY_SELF` as committer, written by the server whatever the request names; the contribution list flags it `inserted_by_person`; an update or deletion by the person of a version anyone else committed is refused with **403**. Shipped, [#4350](https://github.com/FerroHEALTH/FerroEHR/issues/4350) | Give the person's access service `patient/` scopes only and clinicians' applications `user/` scopes, and decide which templates the person may write | Showing the person's entries as theirs to a reader is the presenting application's; the marker it reads is on every version |
| **EHDS [Art. 6](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, requesting rectification online under GDPR Art. 16 | A correction is a new version: the prior version stays in the versioned object's revision history, with the committer and the server's commit time on each ([composition version history](../using-the-api/resources.md#composition-version-history)) | Take the online request in the access service, verify the accuracy with a health professional where appropriate, and commit the correction under a professional's token. Under the Art. 5 rule the person cannot change a professional's version themselves | Receiving the request and deciding it are the controller's (GDPR Art. 16); the repository holds the outcome |
| **EHDS [Art. 7](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, transmission to another provider "without hindrance from the healthcare provider or from the manufacturers", across borders in the exchange format, and one-way to a social security recipient | The REST API, AQL and the EXTRACT export, whose routes are [not admin-gated](../operations-admin-apis.md#ehr-extract-export-and-import), so any authorized caller can take the whole record or a part of it | Transmit the data to the recipient the person names, one-way for a social security recipient (Art. 7(3)), through the exchange format where Art. 7(2) asks for it | Cross-border transmission runs through the infrastructure of Art. 23 (MyHealth@EU and the national contact points), which no CDR is part of; the exchange format is FerroBRIDGE's, as in the Art. 3(2) row |
| **EHDS [Art. 8](https://eur-lex.europa.eu/eli/reg/2025/327/oj)** with **[Art. 11(5)](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, restricting professionals' access, invisible to providers, with access "where necessary in order to protect the vital interests of the data subject", logged | The [restriction of access](../security.md#restriction-of-access-by-health-professionals-ehds-art-8) for a whole EHR or one versioned object, `POST /ferroehr/rest/extensions/v1/admin/access-restriction` and `…/access-restriction/lift`: restricted data reads as absent on every professional route, and no status, header or count says a restriction exists. The role `authz.rbac.emergency_access_role` declaring one of `[audit] emergency_purpose_codes` overrides it, and the access record carries the [emergency mark](../audit.md#emergency-access-ehds-art-115) | Record the restriction on the person's instruction, tell the person that it may affect their care (Art. 8, second paragraph), configure the emergency role and codes, and apply your Member State's rules and safeguards | The grain stops at the versioned object, as the [restriction section](../security.md#restriction-of-access-by-health-professionals-ehds-art-8) explains; the rules and safeguards of the mechanism are the Member State's (Art. 8, fourth paragraph) |
| **EHDS [Art. 9](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, information on every access, including through automatic notifications, available "for at least three years from each date of access" | An [access record](../audit.md) of every read, write and refusal with the elements Art. 9(2)(a) to (c) name; a [subject-scoped ITI-81 read](../audit.md#retrieving-audit-records-iti-81) for one patient under `authz.rbac.subject_audit_role`, which a patient portal can hold without an admin credential; the emergency mark of Art. 11(5); and a boot-time [retention floor](../audit.md#retention-and-who-chooses-it) of three years for every Member State the identifier-scan rules name | Present the trail to the person in the access service, send the automatic notifications, keep the retention at three years or more, and apply any exception your Member State makes under Art. 9(3) | Notifying the person is the access service's (Art. 9(2)); the repository has no channel to the person. A portal built on the subject-scoped read gets the Art. 3(3) filter, while one built on the feeds does not |
| **EHDS [Art. 10](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, a reversible opt-out from access through the access services of Arts. 4 and 12, where a Member State's law provides one, with access to protect vital interests | The [opt-out mark](../security.md#opt-out-from-the-access-services-ehds-art-10) on a whole EHR, `POST /ferroehr/rest/extensions/v1/admin/access-opt-out` and `…/access-opt-out/reverse`, kept with its history: a caller holding one of `authz.rbac.access_service_roles` reads the EHR as absent at every route, query and export, and every other caller, your own care included, reads it as before. An access-service caller declaring one of `[audit] emergency_purpose_codes` and not acting for the person overrides it, marked and recorded. Recording, reversing and the override are access records. Off by default. Shipped, [#4516](https://github.com/FerroHEALTH/FerroEHR/issues/4516) | Declare the roles your access services carry, record and reverse the opt-out on the person's instruction, configure the emergency codes, and apply your Member State's law and its safeguards (Art. 10(2)); where the Member State enforces the opt-out in its own services, leave the mark unused | Whether the right exists at all, and its rules and safeguards, are the Member State's (Art. 10(1) and (2)) |
| **EHDS [Art. 11(1) and (3)](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, professionals' access to the data of persons under their treatment, under the Member State's rules on the categories each category of professional may reach | Per-EHR [`EHR_ACCESS`](../security.md#per-ehr-access-control-ehr_access), role-based access, and [attribute-based policies](../security.md#abac-attribute-based-fine-grained) that see the caller's roles and organisation and the data's category and origin | Write the policies that carry your Member State's rules, and the treatment relationship they turn on | The rules themselves are the Member State's (Art. 11(3)); cross-border access (Art. 11(2) and (4)) runs through the Art. 23 infrastructure |
| **EHDS [Art. 12](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, health professional access services open only to professionals holding recognised electronic identification means | [`[authz.assurance]`](../security.md#assurance-level-and-the-natural-person-on-patient-data) refuses patient data below a minimum eIDAS level, and `[authz.professional]` refuses a client token that names no natural person | Run the identity provider that issues the recognised means, map its levels, and set the minimum | The professional access service and its "user-friendly" presentation are the Member State's and the clinical application's |
| **EHDS [Art. 13(4)](https://eur-lex.europa.eu/eli/reg/2025/327/oj)**, second subparagraph: a registration or update identifies the professional, the provider and the time | Every version carries the server's `time_committed`, the `system_id`, and the committer the caller names; the access record of the write names the authenticated person, with `[authz.professional]` the professional behind a client, and the organisation where the token carries it. The server [checks the committer](../using-the-api/resources.md#contribution) where it can: one that names another login of the caller's token issuer or of the server, or a `PARTY_SELF` from anyone but the person, is refused with **403**. Shipped, [#4520](https://github.com/FerroHEALTH/FerroEHR/issues/4520) | Have your clinical applications name the professional as committer with the login the token carries, since a name only or another authority's identifier cannot be compared with the login and is stored as sent; register the priority categories (Art. 13(1)) under the person's identification data in the Member State of affiliation (Art. 13(3)) | The duty to register, and the data quality requirements Art. 13(4) leaves to implementing acts, rest on the healthcare provider and the Commission |

## Where FerroEHR documents its side

The middle column of each table in the FerroHEALTH book, what the software
provides, points at FerroEHR's own pages:

- **[Compliance overview](index.md):** the legal sources, and what FerroEHR
  ships, act by act and country by country, with the tracker issue for each
  control.
- **[Control matrix](control-matrix.md):** the live status of every declared
  control, generated from the tracker.
- **[Security](../security.md)** and the **[threat model](../threat-model.md):**
  how each control is configured, and the risk that survives it.
- **[Audit trail](../audit.md):** what the access log records and how to read
  it back.
- **[Retention, restriction and objection](retention.md):** the retention
  register, the restriction of processing and the research objection.
- **[Data protection impact assessment](../security/dpia.md)**,
  **[records of processing](../security/records-of-processing.md)** and the
  **[go-live checklist](../security/go-live-checklist.md):** the companion
  guidance for the controller's own documents.
- **[Intended purpose](intended-purpose.md#the-security-environment-ferroehr-assumes):**
  the security environment FerroEHR assumes the deployment provides.

None of these pages tells you whether your deployment satisfies an
obligation. That depends on your legal basis, your organisation, your
infrastructure and your operating practice, none of which a supplier can see.

## The delay of the person's access (EHDS Art. 3(3))

Art. 3(3) lets a Member State delay a person's access to their newly
registered data until a health professional has explained it. The software
part is FerroEHR's: the hold, the release by a professional, the limit that
ends every hold, and the access record of each. The rule is yours to carry:
whether your Member State provides for the delay at all, which categories of
data it covers, how long a hold may last, and which professional releases a
result and when. FerroEHR ships the delay off; see [delayed access by the
person](../security.md#delayed-access-by-the-person-ehds-art-33).
