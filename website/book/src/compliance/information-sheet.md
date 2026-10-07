# Information sheet

The information sheet that accompanies each FerroEHR release, free of charge,
as EHDS Art. 30(1)(d) asks (Regulation (EU) 2025/327). EHDS Art. 38(1) asks
for "concise, complete, correct and clear information that is relevant,
accessible and comprehensible to professional users", and Art. 38(2) lists
what it specifies; each section below is one point of that list. The
[instructions for use](instructions-for-use.md) accompany it.

<!-- toc -->

> [!NOTE]
> This sheet is published in the book of each release, at
> `https://ferroehr.eu/docs/vX.Y.Z/compliance/information-sheet.html`, and
> attached to each GitHub release as `ferroehr-vX.Y.Z-information-sheet.md`,
> a plain-text Markdown file a screen reader reads as text, with that
> release's version and date stamped at the top. The book at `/docs/dev/`
> describes the code on `main`, which is not a release. The article texts are
> those vendored at
> [`docs/law/eu/ehds/text.html`](https://github.com/FerroHEALTH/FerroEHR/blob/main/docs/law/eu/ehds/text.html).
> No conformity assessment has been carried out and no EU declaration of
> conformity exists; nothing on this sheet says that FerroEHR meets the
> Regulation.

## (a) The manufacturer

EHDS Art. 38(2)(a): "the identity, registered trade name or registered
trademark, and contact details of the manufacturer and, where applicable, of
its authorised representative".

| | |
|---|---|
| Manufacturer | Cadasto B.V. |
| Postal address | Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands |
| Single point of contact | [info@cadasto.com](mailto:info@cadasto.com) |
| Website | <https://www.cadasto.com/contact/> |
| Authorised representative | none: Cadasto B.V. is established in the Union |
| Vulnerability reports | GitHub private vulnerability reporting, or info@cadasto.com ([`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#reporting-a-vulnerability)) |
| Complaints and serious incidents | info@cadasto.com ([complaints, incidents and vulnerabilities](post-market.md)) |

The running server names the manufacturer in the same words on its startup
banner, in `ferroehr --version` and on `GET /management/info`
([the manufacturer and the deployment report](../operations.md#the-manufacturer-and-the-deployment-report)).

## (b) Name, version and release date

EHDS Art. 38(2)(b): "the name and version of the EHR system and date of its
release".

- **Name:** FerroEHR, one of the two products of the EHR system Cadasto B.V.
  declares. The other is FerroBRIDGE, which ships the European
  interoperability software component and publishes its own version, date and
  information sheet with its releases. The two meet over the openEHR REST API.
- **Version:** the release this sheet belongs to: the tag `vX.Y.Z`, the version
  `ferroehr --version` prints, and `server_version` on
  `GET /ferroehr/rest/status`.
- **Release date:** the date on the release's heading in the
  [changelog](https://github.com/FerroHEALTH/FerroEHR/blob/main/CHANGELOG.md),
  `## [X.Y.Z] - YYYY-MM-DD`.
- **Support period:** five years from the month of the release date; the end
  month is in the release notes and in
  [`SECURITY.md`](https://github.com/FerroHEALTH/FerroEHR/blob/main/SECURITY.md#supported-versions).

## (c) Intended purpose

EHDS Art. 38(2)(c): "the intended purpose of the EHR system".

FerroEHR stores, versions and queries the structured health records of one
healthcare provider, and serves them to the software that provider's health
professionals and patients use, through the openEHR REST API and AQL. It
records every access to those records in its access log, the European logging
software component. It has no clinical user interface of its own, and it is
not intended for any medical device purpose. It runs one instance per
organisation, operated by the organisation itself, by a processor on its
behalf, or by Cadasto B.V. as a hosted service.

The full statement, with the users, the environment and the foreseeable
misuse, is the [intended purpose](intended-purpose.md).

## (d) The categories of electronic health data

EHDS Art. 38(2)(d): "the categories of electronic health data that the EHR
system has been designed to process".

- **Clinical content:** the priority categories of EHDS Art. 14(1), "(a)
  patient summaries; (b) electronic prescriptions; (c) electronic
  dispensations; (d) medical imaging studies and related imaging reports; (e)
  medical test results, including laboratory and other diagnostic results and
  related reports; and (f) discharge reports", and any category a Member State
  adds in national law, as far as the operational templates the deploying
  organisation loads describe them. Imaging studies are held as attachments or
  as references to an imaging archive; FerroEHR implements no DICOM image
  transfer. The deployment declares which templates carry which category in
  the [`[audit.categories]`](../installation/config-audit.md#auditcategories)
  map.
- **Identities**, in a separate demographic domain: persons, organisations and
  roles, and their identifiers.
- **The link** between a record and its subject, in a separate linkage domain.
- **The access log:** who accessed which record, when, from which origin and
  in which priority category, naming health professionals and patients.

## (e) Standards, formats and specifications

EHDS Art. 38(2)(e): "the standards, formats and specifications supported by
the EHR system and versions of those standards, formats and specifications".

| Standard or specification | Version | Used for |
|---|---|---|
| openEHR Reference Model (RM) | 1.2.0, and 1.1.0 | the record content |
| openEHR BASE | 1.3.0, and 1.2.0 | foundation and base types |
| openEHR Archetype Model (AM), ADL | 1.4 and 2.4.0 | archetypes and templates |
| openEHR operational templates (OPT) | 1.4 | template upload and validation |
| openEHR Terminology (TERM) | 3.1.0 | the openEHR terminology |
| openEHR ITS-REST | Release-1.1.0 | the REST API |
| openEHR Archetype Query Language (AQL) | 1.1.0 | queries |
| openEHR canonical JSON and XML (ITS-JSON, ITS-XML) | ITS-XML Release-1.0.2 and Release-2.0.0 schemas | serialisation |
| openEHR simplified formats (Web Template, FLAT, STRUCTURED) | as ITS-REST Release-1.1.0 defines them | simplified data entry and retrieval |
| IHE ATNA: Record Audit Event (ITI-20), Authenticate Node (ITI-19), Retrieve ATNA Audit Event (ITI-81); IHE Basic Audit Log Patterns (BALP) | the profile revision is not pinned | the access log |
| DICOM PS3.15 §A.5 audit message | the edition is not pinned | the access log over syslog |
| HL7 FHIR `AuditEvent` | R4 | the access log as FHIR |
| Syslog | RFC 5424, over UDP or TLS | the access-log feed |
| OAuth 2.0 bearer tokens, JWT, OpenID Connect | RFC 6750, RFC 7519, OpenID Connect Core 1.0 | authentication |
| SMART App Launch | the version is not pinned; PKCE per RFC 7636 | application launch and scopes |
| TLS | 1.3 by default | transport security |
| PostgreSQL | 18 | the database the deployment provides |

Which openEHR generation a deployment serves is a configuration choice
([choosing a specification generation](../installation/index.md#choosing-a-specification-generation)).
The conformance record measured against the openEHR specifications is on the
[conformance](../conformance.md) page; it measures the openEHR
specifications, not the EHDS. The European electronic health record exchange
format of EHDS Art. 15 has not been adopted, and FerroBRIDGE, not FerroEHR,
carries it.
