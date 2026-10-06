# Shared responsibility

Almost every obligation in EU and national health-data law rests on the
controller, and where FerroEHR is operated on that controller's behalf, on the
processor. A repository supplies technical measures. It cannot hold a legal
basis, sign a processing agreement, notify a supervisory authority or run a
management system.

This page draws that line obligation by obligation, so you can see at a glance
which part of the work the software has already done and which part is still
yours. No openEHR specification governs any of it; the division below follows
the legal texts each row links.

<!-- toc -->

## How to read the tables

Each row names one obligation and links its official source. The middle column
is what FerroEHR provides, linked to the page that documents it, or to the open
issue when the control is planned rather than shipped. The right column is the
work that stays with the deploying organisation.

"Nothing" in the middle column is a real answer and appears wherever it is
the true one.

> [!NOTE]
> Cadasto B.V. is the manufacturer of each FerroEHR release. Whether it is
> also your processor depends on who runs the deployment:
>
> - **Self-hosted.** You run the deployment, or a processor of your choosing
>   runs it for you. Cadasto B.V. supplies the software and processes no
>   personal data on your behalf, so it is not your processor (GDPR Art. 4(8)
>   defines a processor as one "which processes personal data on behalf of the
>   controller").
> - **Hosted by Cadasto B.V.** Cadasto B.V. runs the deployment as a service
>   for you. It is then also your processor, and Art. 28(3) has that processing
>   "governed by a contract or other legal act" binding it with regard to you.
>
> In both cases the one thing a deployment sends out by default is the
> [usage report](../usage-report.md): a random instance id, the version, the
> licence grant type and coarse performance figures, with no patient data.
> Cadasto B.V. receives it as controller for its own purposes, and its
> collector also stores the IP address each report comes from. Where a row
> says "the processor", it means whoever runs the deployment for you.

## GDPR

Every row below cites
[Regulation (EU) 2016/679](https://eur-lex.europa.eu/eli/reg/2016/679/oj). The
duties in it belong to the controller and the processor. The middle column is
only the technical measure FerroEHR supplies toward one of them.

| Obligation | What FerroEHR provides | What the deploying organisation does |
|---|---|---|
| [Art. 5(1)(e)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) storage limitation | Audit-trail retention as a configured period, and irreversible [physical deletion](../operations-admin-apis.md#physical-deletion) of an EHR through the admin API | Set the retention schedule and execute it; openEHR versions are append-only until you delete the record |
| [Art. 5(2)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) accountability | An [audit trail](../audit.md) of every access, openEHR's own contribution and audit chain on every write, and a published [conformance record](../conformance.md) | Retain the evidence and be able to produce it on demand |
| [Art. 6 and Art. 9(2)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) legal basis and the condition for health data | Nothing. Software cannot hold a legal basis | Establish the basis and the Art. 9(2) condition, per purpose, before data is entered |
| [Art. 24 and Art. 25](https://eur-lex.europa.eu/eli/reg/2016/679/oj) responsibility, and protection by design and by default | Deny-by-default [authorization](../security.md#authorization), per-EHR access settings, tenancy that fails closed, audit on by default | Choose the restrictive settings, and document why the chosen configuration is appropriate |
| [Art. 28(3)(e) to (h)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) what a processor's contract must let it do | The rights operations in the rows below for (e); the trail, the [threat model](../threat-model.md) and the published control documentation for (f); [EHR Extract export](../beyond-core/messaging.md#exporting-an-ehr) and [physical deletion](../operations-admin-apis.md#physical-deletion) at the end of service for (g); `GET {base}/admin/config`, the trail and verifiable [release artifacts](../verifying-releases.md) as the evidence (h) asks for | Conclude the processing agreement with whoever operates the deployment, and audit them |
| [Art. 30](https://eur-lex.europa.eu/eli/reg/2016/679/oj) records of processing activities | The effective configuration as a redacted tree at `GET {base}/admin/config`, and this book as a description of what the software does | Write and maintain the record; only you know the purposes, the recipients and the transfers |
| [Art. 32](https://eur-lex.europa.eu/eli/reg/2016/679/oj) security of processing | TLS 1.3 with optional [mutual authentication](../audit.md#node-authentication-iti-19-mutual-tls), [authentication and access control](../security.md), per-version [signing](../signing/index.md), a tamper-evident audit chain, [domain-separated database roles](../security.md#the-pseudonymisation-boundary) | Supply everything below the application: the database, its backups, the network, the platform. See [Cluster hardening](../installation/kubernetes-hardening.md) |
| [Art. 32(1)(d)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) regularly testing the measures | A [storage-integrity sweep and rebuild](../operations-admin-apis.md#storage-integrity), an [audit-chain verification query](../audit.md#tamper-evidence), and a [conformance suite](../conformance.md) that runs against your own server | Schedule the checks, alert on their output, and test your restore |
| [Art. 33(3)(a) and 34(3)(a)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) breach notification, and the measures that remove the duty to tell patients | The evidence a breach assessment needs: who read what, when, per patient and per agent, and whether the trail itself is intact | Detect, assess and notify within the deadlines. 34(3)(a) lifts the duty to inform patients only where the measures were applied to the data affected, and the application seals national identifiers only, so encryption of clinical content at rest is the database's and the disk's. The regulation says nothing about encryption at rest; the measure is yours to choose |
| [Art. 12(3)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) act on a rights request within one month, electronically | Every right below is served by an API call, so the answer is produced in the run that handles the request | Start the clock at receipt, verify the requester, and use the two-month extension only with the reasons the article asks for |
| [Art. 15 and 20](https://eur-lex.europa.eu/eli/reg/2016/679/oj) access, a copy, and portability | The full record over the openEHR REST API in canonical JSON or XML, the [simplified FLAT and STRUCTURED formats](../using-the-api/content-negotiation.md#simplified-formats-flat-and-structured), and [EHR Extract export](../beyond-core/messaging.md#exporting-an-ehr) for a whole record, which another openEHR system imports directly; the recipients of 15(1)(c) from the [per-patient trail search](../audit.md#retrieving-audit-records-iti-81) | Authenticate the data subject and build the patient-facing route. The purposes and the storage period of 15(1) come from your own records, and the portability right reaches consent-based or contract-based processing only (20(3) excludes a public-interest task) |
| [Art. 16 and 17](https://eur-lex.europa.eu/eli/reg/2016/679/oj) rectification and erasure | Versioned correction with the prior version retained, which is the supplementary statement Art. 16 allows, and [physical, irreversible deletion](../operations-admin-apis.md#physical-deletion) of an EHR and everything it owns, over the primary and the cold archival tier alike | Decide how an erasure request interacts with the medical record-keeping duty and with the Art. 17(3) grounds, record the decision, and reach the backups and the copies outside the CDR |
| [Art. 18 and 21](https://eur-lex.europa.eu/eli/reg/2016/679/oj) restriction and objection | A [restriction register](retention.md#restriction-of-processing) at whole-EHR or single-object grain, with the marked object refused on every read, query, export, event and write path while its stored rows stay untouched, and its lift stamped rather than erased ([#3324](https://github.com/FerroHEALTH/FerroEHR/issues/3324)); a [research objection](retention.md#objection-to-research-processing) under Art. 21(6) that removes the record from population queries, exports and the event stream while leaving reads for care untouched ([#3325](https://github.com/FerroHEALTH/FerroEHR/issues/3325)) | Decide when to set them, record the ground, hold the restriction in the systems around the CDR, and tell the subject before lifting it |
| [Art. 19](https://eur-lex.europa.eu/eli/reg/2016/679/oj) telling each recipient about a rectification, erasure or restriction | An access record for every read and export naming the agent, the patient, the action, the outcome and the time, so the recipient list is answerable from the [trail](../audit.md#retrieving-audit-records-iti-81) | Send the communications, and name the recipients to the subject on request. The trail records who received data; it notifies nobody, and the regulation sets no retention period for an access log, so how far back the list reaches is the retention you configure |
| [Art. 35](https://eur-lex.europa.eu/eli/reg/2016/679/oj) data protection impact assessment | A [DPIA page](../security/dpia.md) to assess against (processing description, data categories per schema, roles, retention, risk register, shipped controls by issue), [records of processing](../security/records-of-processing.md) pre-filled with what the software does, and a [go-live checklist](../security/go-live-checklist.md) | Run the DPIA and keep it current. It is the controller's, and no supplier document replaces it |
| [Art. 4(5)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) pseudonymisation | Clinical data, demographic data and the EHR id / subject cross-reference live in three separate schemas with non-overlapping `NOINHERIT` roles, and the server refuses to boot if a role reaches across ([the boundary](../security.md#the-pseudonymisation-boundary)). The cross-reference — which party and which subject identifier name which EHR, the openEHR Service Model's EHR Index — is resolved only through the `linkage` pool, every resolution, merge and split an audited linkage access record ([#3158](https://github.com/FerroHEALTH/FerroEHR/issues/3158), [#3345](https://github.com/FerroHEALTH/FerroEHR/issues/3345)); the clinical schema keeps only the guarded `EHR_STATUS` subject pair the openEHR wire binds to, and deleting an EHR removes the cross-reference rows naming it | Give the demographic and linkage pools their own DSNs, restrict who may resolve, and hold any additional information outside the CDR to the same standard |
| [Art. 89(1)](https://eur-lex.europa.eu/eli/reg/2016/679/oj) research safeguards, and anonymising where the purpose allows it | [Cohort queries](../querying-aql.md#cohort-queries-across-the-pseudonymisation-boundary) that answer a research question as an aggregate, withheld below the configured small-cell threshold; secondary use leaves the repository through the FerroBRIDGE product to the OMOP CDM, pseudonymised per permit there; the repository's side of that path (the restriction, objection and retention marks every export honours, a resumable batch export, an access event per export) is planned in [#3379](https://github.com/FerroHEALTH/FerroEHR/issues/3379) | Decide whether the purpose can be met without identification and take that route where it can. Until the export lands, secondary use runs on the primary store |

## EHDS

[Regulation (EU) 2025/327](https://eur-lex.europa.eu/eli/reg/2025/327/oj)
is in force, and its operative obligations apply from the dates its own final
provisions carry. No row below claims conformity with any of them.

| Obligation | What FerroEHR provides | What the deploying organisation does |
|---|---|---|
| [Chapter II](https://eur-lex.europa.eu/eli/reg/2025/327/oj), primary use and the patient's sight of who accessed their data | An access trail of every read, write and refusal, [searchable by patient and by agent](../audit.md#retrieving-audit-records-iti-81), and a [subject-scoped read grant](../installation/config-auth.md#authzrbac) sized for a patient portal | Build the patient-facing access route on that grant and authenticate the person; the product serves one subject's log to it, never every patient's |
| [Chapter III](https://eur-lex.europa.eu/eli/reg/2025/327/oj), EHR systems: the two harmonised software components, technical documentation, the information sheet and instructions for use, the declaration of conformity | The logging component, with a status and evidence per Annex II requirement on the [EHDS readiness page](ehds-readiness.md); the [technical documentation](technical-documentation.md), the [information sheet](information-sheet.md) and the [instructions for use](instructions-for-use.md) of each release, kept by Cadasto B.V. as manufacturer. The interoperability component is FerroBRIDGE's, and the exchange format waits on implementing acts under Article 15(1) | Configure the deployment as the instructions for use say. A self-hosted deployment of an unmodified release puts Cadasto B.V.'s product into service and leaves the manufacturer's duties with Cadasto B.V.; in a deployment Cadasto B.V. hosts, it carries them as manufacturer and operator |
| [Chapter IV](https://eur-lex.europa.eu/eli/reg/2025/327/oj), secondary use | [AQL](../querying-aql.md) over the stored record and a [change-event outbox](../beyond-core/amqp.md); the batch export the FerroBRIDGE OMOP load consumes, filtered by the restriction and objection marks, is planned in [#3379](https://github.com/FerroHEALTH/FerroEHR/issues/3379) | Deal with the health data access body and carry the data holder's duties |

## CRA

[Regulation (EU) 2024/2847](https://eur-lex.europa.eu/eli/reg/2024/2847/oj)
puts its duties on the manufacturer of a product with digital elements, which
for each FerroEHR release is Cadasto B.V. The [Cyber Resilience Act](cra.md)
page states the position.

| Obligation | What FerroEHR provides | What the deploying organisation does |
|---|---|---|
| [Art. 13 and Annex I](https://eur-lex.europa.eu/eli/reg/2024/2847/oj), the essential cybersecurity requirements and the manufacturer's obligations, from 11 December 2027 | The [CRA risk assessment](cra-risk-assessment.md), point by point against Annex I, with the evidence and the open work | Run a supported release, apply security updates, and keep the security environment the [intended purpose](intended-purpose.md#the-security-environment-ferroehr-assumes) assumes |
| [Art. 14](https://eur-lex.europa.eu/eli/reg/2024/2847/oj), reporting actively exploited vulnerabilities and severe incidents, since 11 September 2026 | Cadasto B.V. notifies the CSIRT and ENISA ([complaints, incidents and vulnerabilities](post-market.md)) | Report what you see to Cadasto B.V.; your own NIS2 and GDPR notifications stay yours |
| [Art. 13(8)](https://eur-lex.europa.eu/eli/reg/2024/2847/oj), the support period | Five years per release, the end date in the release notes | Upgrade before the end date, and to the newest release when it carries a security fix |

## National law

The sections above apply to every EU deployment. This one is a single
country's law on top of them, three countries so far: the division a
deployment reads is "the EU layer, plus my own jurisdiction".
The compliance overview says what adding another takes
([National law](index.md#national-law)).

### The Netherlands

| Obligation | What FerroEHR provides | What the deploying organisation does |
|---|---|---|
| [UAVG Art. 30](https://wetten.overheid.nl/BWBR0040940), the exception for health data | Access control at the record and attribute level, with every use audited | Establish that your processing falls inside the exception, per role and per purpose |
| [UAVG Art. 46](https://wetten.overheid.nl/BWBR0040940), processing a national identification number | National identifiers sealed at rest under the instance's identifier-protection key in the demographic domain, resolved only through the demographic role, every resolution recorded as a linkage access without the value | Hold the statutory authorisation before a BSN enters the store, and restrict who may resolve |
| [Wabvpz Art. 4 to 9](https://wetten.overheid.nl/BWBR0023864), use and verification of the BSN | Nothing. FerroEHR performs no BSN verification and consults no index | Verify identity and the BSN in your own systems before data reaches the CDR |
| [Wabvpz Art. 15d](https://wetten.overheid.nl/BWBR0023864), electronic access and copy for the patient | The full record over the REST API, and [EHR Extract export](../beyond-core/messaging.md) | Authenticate the patient and build the route; the CDR has no patient-facing interface |
| [Wabvpz Art. 15e](https://wetten.overheid.nl/BWBR0023864), a record of who made data available and who consulted it | An [ATNA trail](../audit.md) recording the agent, the patient, the action, the outcome and the time, retrievable per patient | Render it for the patient, set retention, and review it |
| [BW Book 7, Art. 454](https://wetten.overheid.nl/BWBR0005290), the medical treatment contract's record-keeping duty | Append-only version history, so a correction never destroys the prior version | Set the retention schedule the article requires, and reconcile it with erasure requests |

### The Netherlands: NEN

The [NEN 7510 family](https://www.nen.nl/zorg-welzijn/ict-in-de-zorg/informatiebeveiliging-in-de-zorg)
is where the split is sharpest. A management-system standard cannot be met by
a product at all.

| Obligation | What FerroEHR provides | What the deploying organisation does |
|---|---|---|
| [NEN 7510-1](https://www.nen.nl/nen-7510-1-2024-nl-331311), the information security management system | Technical controls an ISMS can point at, each documented with its residual risk in the [threat model](../threat-model.md) | Run the ISMS: scope, risk assessment, policy, internal audit, management review |
| [NEN 7510-2](https://www.nen.nl/nen-7510-2-2024-nl-331314), the controls | [Access control](../security.md), [audit logging](../audit.md), cryptography in transit and for [version signatures](../signing/index.md), [supply-chain verification](../verifying-releases.md) | Everything organisational: personnel, physical security, supplier management, continuity |
| [Certification](https://www.nen.nl/certificatie-en-keurmerken-nen-7510) against NEN 7510 | Nothing. A product cannot be certified against a management-system standard, and FerroEHR makes no such claim | Obtain and maintain the certificate for your organisation |
| [NEN 7512](https://www.nen.nl/nen-7512-2022-nl-297137), the trust basis for data exchange | [Mutually authenticated TLS](../audit.md#node-authentication-iti-19-mutual-tls), OAuth2 and OIDC with an [enterprise identity provider](../identity-providers.md), [SMART App Launch](../smart-app-launch.md) | Agree the trust basis with each counterparty, and operate the certificate estate |
| [NEN 7513](https://www.nen.nl/nen-7513-2018-nl-245399), logging actions on electronic patient records | An [audit trail](../audit.md) of every operation including refusals, in FHIR `AuditEvent` and DICOM PS3.15 form, hash-chained in the database | Map the recorded fields onto the standard's own list, set retention, and review the trail |

### Germany

| Obligation | What FerroEHR provides | What the deploying organisation does |
|---|---|---|
| [BDSG § 22 Abs. 2](https://www.gesetze-im-internet.de/bdsg_2018/__22.html), appropriate and specific measures for health data: traceability, access restriction, pseudonymisation, encryption | Versioned writes with contribution and audit, an [access trail](../audit.md), deny-by-default [authorization](../security.md#authorization), the pseudonymisation boundary, TLS and sealed identifiers | Choose the measures, establish the lit. b ground, and keep the processing under persons bound by professional secrecy |
| [BDSG § 27 Abs. 3](https://www.gesetze-im-internet.de/bdsg_2018/__27.html), identifying characteristics stored separately for research, rejoined only as the purpose requires | Separate clinical, demographic and linkage schemas, and [cohort queries](../querying-aql.md#cohort-queries-across-the-pseudonymisation-boundary) that cross on identifiers only | Decide when to anonymise, and hold the balancing test |
| [SGB V §§ 346 to 348](https://www.gesetze-im-internet.de/sgb_5/__347.html), writing treatment data into the ePA once it is held in interoperable form | Template-structured records, the REST API and [EHR Extract export](../beyond-core/messaging.md) | Operate the transport into the ePA, the connector and the information objects |
| [SGB V § 339 Abs. 3](https://www.gesetze-im-internet.de/sgb_5/__339.html) and [§ 352](https://www.gesetze-im-internet.de/sgb_5/__352.html), credential-bound access with a log of who accessed what, under a closed role matrix | Role- and attribute-based authorization and an [access trail](../audit.md) naming agent, roles, patient, action and outcome | Bind the identity provider to the HBA and SMC-B; the sections bind the ePA, which the CDR is not |
| [SGB V § 309](https://www.gesetze-im-internet.de/sgb_5/__309.html), the TI access log with attempts, three years' retention and deletion on expiry | A trail that records attempts and refusals, `retention_days` and the [retention reaper](../audit.md#retention-and-who-chooses-it) | Set the retention owed; the section binds TI application controllers, not a CDR outside the TI |
| [GDNG § 6](https://www.gesetze-im-internet.de/gdng/__6.html), own-data secondary use under pseudonymisation, a rights-and-roles concept, logging and a thirty-year limit | Per-domain roles, cohort queries with small-cell suppression, an access record per query with `purpose` and `legal_basis`; the export to the FerroBRIDGE OMOP load, where pseudonymisation per permit happens, is planned in [#3379](https://github.com/FerroHEALTH/FerroEHR/issues/3379) | Write the rights-and-roles concept, publish the purposes, run the clock, answer subjects from the trail |
| [§ 203 StGB Abs. 3 and 4](https://www.gesetze-im-internet.de/stgb/__203.html), necessity-bounded access for those who keep the systems running, and the duty to bind them to secrecy | Separate operational [surfaces](../security.md#operational-surfaces-what-is-reachable-and-by-whom), audited admin reads, one database role per domain | Bind operators and subcontractors to secrecy in writing, and route support so it needs no standing read of clinical content |

### Switzerland

Switzerland is not an EU member state: the GDPR rows above do not apply to a
Swiss deployment, and the [DSG](https://www.fedlex.admin.ch/eli/cc/2022/491/de)
takes their place.

| Obligation | What FerroEHR provides | What the deploying organisation does |
|---|---|---|
| [DSG Art. 7](https://www.fedlex.admin.ch/eli/cc/2022/491/de#art_7), data protection by design and by default | Deny-by-default [authorization](../security.md#authorization), a `production` [deployment profile](../installation/configuration.md#deployment_profile) that refuses missing separations | Choose the restrictive settings where the shipped default favours compatibility |
| [DSG Art. 8](https://www.fedlex.admin.ch/eli/cc/2022/491/de#art_8) with [DSV Art. 3](https://www.fedlex.admin.ch/eli/cc/2022/568/de#art_3), the minimum security measures | Need-to-know authorization, one database role per domain, TLS 1.3, attributed versioned writes, an [access trail](../audit.md) with refusals, [signed releases](../verifying-releases.md) | Backup and restore, patching, breach detection and everything below the application |
| [DSV Art. 4](https://www.fedlex.admin.ch/eli/cc/2022/568/de#art_4), logging including reads, kept at least a year separately from the processing system | The trail with every operation and its actor, time and outcome, and [forwarding sinks](../audit.md#getting-the-log-out) that put a copy outside the CDR | Forward the trail, set the retention at a year or more, restrict who reads it |
| [DSG Art. 12](https://www.fedlex.admin.ch/eli/cc/2022/491/de#art_12), the register of processing activities | The effective configuration at `GET {base}/admin/config` and [records of processing](../security/records-of-processing.md) pre-filled with what the software does | Write and maintain the register; DSV Art. 24 leaves no small-organisation exemption for a clinical repository |
| [DSG Art. 22](https://www.fedlex.admin.ch/eli/cc/2022/491/de#art_22), the impact assessment for large-scale processing of sensitive data | The [DPIA page](../security/dpia.md) with the technical description, the risk register and the controls by issue | Run the assessment; it is the controller's |
| [DSG Art. 25](https://www.fedlex.admin.ch/eli/cc/2022/491/de#art_25) and [Art. 28](https://www.fedlex.admin.ch/eli/cc/2022/491/de#art_28), the right of access within 30 days and data portability in a common electronic format | The full record over the REST API, the [EHR Extract](../beyond-core/messaging.md), the published openEHR formats, and a [per-patient search of the trail](../audit.md#retrieving-audit-records-iti-81) for the recipients | Identify the requester, render the answer understandably, route it, and meet the deadline |
| [DSG Art. 31 Abs. 2 lit. e](https://www.fedlex.admin.ch/eli/cc/2022/491/de#art_31), research on anonymised data, with measures against identifiability meanwhile | Separate clinical, demographic and linkage schemas and [cohort queries](../querying-aql.md#cohort-queries-across-the-pseudonymisation-boundary) with small-cell suppression; the export to the FerroBRIDGE OMOP load, where pseudonymisation per permit happens, is planned in [#3379](https://github.com/FerroHEALTH/FerroEHR/issues/3379) | Decide when anonymisation is possible and hold the research ground |
| [EPDG Art. 10](https://www.fedlex.admin.ch/eli/cc/2017/203/de#art_10) and [EPDV Art. 10 and 12](https://www.fedlex.admin.ch/eli/cc/2017/204/de#art_10), the certified community's logging, storage, encryption and residency duties | [IHE ATNA](../audit.md) events over ITI-20 with ITI-19 mutual TLS, ITI-81 retrieval, the record in published formats, a self-hosted deployment | Feed the EPD through a certified community; the duties bind the community, which the CDR is not |

## What this page does not do

It does not tell you whether your deployment satisfies any of these
obligations. That answer depends on your legal basis, your organisation, your
infrastructure and your operating practice, none of which a supplier can see.

The companion guidance is written: the [DPIA page](../security/dpia.md),
the [records of processing](../security/records-of-processing.md) and the
[go-live checklist](../security/go-live-checklist.md). Beside them, the
[compliance overview](index.md) carries the legal sources, the
[control matrix](control-matrix.md) carries the live status of every declared
control, and the [threat model](../threat-model.md) carries the risk that
survives each one.
