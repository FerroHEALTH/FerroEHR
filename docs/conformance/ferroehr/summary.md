# Veredictum results

| | |
| --- | --- |
| **Server** | http://127.0.0.1:8080/ferroehr/rest/openehr/v1 |
| **Instrument** | veredictum 0.3.3, pre-approval: not yet approved by the openEHR SEC |
| **Document** | `catalogue` |

## Summary

| Part | Passed | Failed | Warning | Errored | Skipped | Total |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| BASE | 2610 | 0 | 0 | 0 | 0 | 2610 |
| OPTIONS | 256 | 0 | 0 | 0 | 116 | 372 |
| **Every row** | **2866** | **0** | **0** | **0** | **116** | **2982** |

> [!NOTE]
> A **failed** row counts against the server. A **warning** row is a SHOULD the server did not follow, and it never counts against the server. An **errored** row is inconclusive when its record names the instrument, which could not judge it, and counts against the server when its record names the server or no one. A **skipped** row was not run.

## BASE

What every claimant of a capability must do. A case no option arm names is BASE.

### EHR

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| EHR | 75 | 0 | 0 | 0 | 0 |
| EHR\_STATUS | 57 | 0 | 0 | 0 | 0 |
| COMPOSITION | 154 | 0 | 0 | 0 | 0 |
| DIRECTORY | 75 | 0 | 0 | 0 | 0 |
| CONTRIBUTION | 64 | 0 | 0 | 0 | 0 |
| Item tags | 71 | 0 | 0 | 0 | 0 |

#### EHR

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Create and read | 66 | 0 | 0 | 0 | 0 |
| Committal headers | 1 | 0 | 0 | 0 | 0 |
| Bulk load | 8 | 0 | 0 | 0 | 0 |

#### EHR\_STATUS

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Read | 14 | 0 | 0 | 0 | 0 |
| Update | 21 | 0 | 0 | 0 | 0 |
| Versioning and history | 18 | 0 | 0 | 0 | 0 |
| Flags | 4 | 0 | 0 | 0 | 0 |

#### COMPOSITION

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Create | 25 | 0 | 0 | 0 | 0 |
| Read | 21 | 0 | 0 | 0 | 0 |
| Update | 25 | 0 | 0 | 0 | 0 |
| Delete | 19 | 0 | 0 | 0 | 0 |
| Versioning and history | 20 | 0 | 0 | 0 | 0 |
| Refusals | 44 | 0 | 0 | 0 | 0 |

#### DIRECTORY

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Create | 12 | 0 | 0 | 0 | 0 |
| Read | 24 | 0 | 0 | 0 | 0 |
| Update | 18 | 0 | 0 | 0 | 0 |
| Delete | 8 | 0 | 0 | 0 | 0 |
| Refusals | 13 | 0 | 0 | 0 | 0 |

#### CONTRIBUTION

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Create | 29 | 0 | 0 | 0 | 0 |
| Read | 11 | 0 | 0 | 0 | 0 |
| Refusals | 24 | 0 | 0 | 0 | 0 |

#### Item tags

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| EHR tags | 10 | 0 | 0 | 0 | 0 |
| EHR\_STATUS tags | 22 | 0 | 0 | 0 | 0 |
| COMPOSITION tags | 39 | 0 | 0 | 0 | 0 |

### Definitions

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Templates | 61 | 0 | 0 | 0 | 0 |

#### Templates

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| ADL 1.4 templates | 31 | 0 | 0 | 0 | 0 |
| ADL 2 templates | 30 | 0 | 0 | 0 | 0 |

### Query

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| AQL and stored queries | 189 | 0 | 0 | 0 | 0 |

#### AQL and stored queries

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Ad-hoc AQL execution | 12 | 0 | 0 | 0 | 0 |
| AQL semantics | 82 | 0 | 0 | 0 | 0 |
| AQL conformance corpus | 48 | 0 | 0 | 0 | 0 |
| Stored queries | 25 | 0 | 0 | 0 | 0 |
| Refusals | 22 | 0 | 0 | 0 | 0 |

### Demographic

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Demographic | 306 | 0 | 0 | 0 | 0 |

#### Demographic

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Parties | 140 | 0 | 0 | 0 | 0 |
| Versioned party | 12 | 0 | 0 | 0 | 0 |
| Demographic CONTRIBUTION | 13 | 0 | 0 | 0 | 0 |
| Item tags | 83 | 0 | 0 | 0 | 0 |
| Refusals | 58 | 0 | 0 | 0 | 0 |

### Content validation

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Content validation | 1548 | 0 | 0 | 0 | 0 |

#### Content validation

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Structure and cardinality (ADL 1.4) | 367 | 0 | 0 | 0 | 0 |
| Structure and cardinality (ADL 2) | 2 | 0 | 0 | 0 | 0 |
| Data types (ADL 1.4) | 822 | 0 | 0 | 0 | 0 |
| Data types (ADL 2) | 58 | 0 | 0 | 0 | 0 |
| Interval data types (ADL 1.4) | 274 | 0 | 0 | 0 | 0 |
| Interval data types (ADL 2) | 25 | 0 | 0 | 0 | 0 |

### Simplified formats

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Simplified formats | 0 | 0 | 0 | 0 | 0 |

### Admin

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Admin | 1 | 0 | 0 | 0 | 0 |

#### Admin

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Refusals | 1 | 0 | 0 | 0 | 0 |

### Security and privacy

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Security | 1 | 0 | 0 | 0 | 0 |

#### Security

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Anonymous EHRs | 1 | 0 | 0 | 0 | 0 |

### SMART on openEHR

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| SMART | 5 | 0 | 0 | 0 | 0 |

#### SMART

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| Discovery | 1 | 0 | 0 | 0 | 0 |
| Resource scopes | 4 | 0 | 0 | 0 | 0 |

### System

| Band | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| System | 3 | 0 | 0 | 0 | 0 |

#### System

| Section | Passed | Failed | Warning | Errored | Skipped |
| --- | ---: | ---: | ---: | ---: | ---: |
| OPTIONS | 3 | 0 | 0 | 0 | 0 |

## OPTIONS

What the statement selected. Each option is labelled with its kind: **Recommended (SHOULD)**, which a server may decline with a warning row; **Optional (MAY)**, behaviour the release leaves to the implementation; or **Spec defect**, an option only because the released text is defective. The statement selects one arm of each, and the other arms are skipped.

### EHR

#### EHR

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `ehr-xml-read` | Optional (MAY) | `ehr-xml-supported` | 4 | 0 | 0 | 0 | 4 |
| `ehr-xml-write` | Optional (MAY) | `ehr-xml-write-accepted` | 2 | 0 | 0 | 0 | 1 |
| `ehr-status-flat-write` | `@SpecDiverge` | `ehr-status-flat-write-refused` | 1 | 0 | 0 | 0 | 0 |
| `ehr-status-structured-write` | `@SpecDiverge` | `ehr-status-structured-write-refused` | 1 | 0 | 0 | 0 | 0 |

<details>
<summary>What these options decide</summary>

- **`ehr-xml-read`** (Canonical XML): Whether the service serves the EHR resource in canonical XML, on a read and as the representation of a create, when a client asks for it with Accept application/xml.
- **`ehr-xml-write`** (Canonical XML): Whether the service processes an EHR\_STATUS sent as canonical XML on the EHR creates. No published document schema reaches EHR\_STATUS: the global \`items\` element (LOCATABLE) of documents/Structure.xsd does not include Ehr.xsd, which only the 2.0.0 lineage publishes, and no released sentence names the root a service accepts (upstream report \#759). The accepting arm sends an \`items\` root typed EHR\_STATUS in that lineage's namespace, so its workflows rest on that report.
- **`ehr-status-flat-write`** (Simplified Formats and Web Template): Whether the service processes an EHR\_STATUS sent with Content-Type application/openehr.wt.flat+json on the EHR creates and the EHR\_STATUS update. The release defines no EHR\_STATUS form in the Simplified Flat JSON format (upstream report \#352), so the accepting arm has no document. Upstream report [#352](https://github.com/Vernum-Projecten/Veredictum/issues/352): ITS-REST declares the Simplified Formats for EHR\_STATUS, FOLDER and the PARTY kinds, which the Simplified Formats specification never maps (open).
- **`ehr-status-structured-write`** (Simplified Formats and Web Template): Whether the service processes an EHR\_STATUS sent with Content-Type application/openehr.wt.structured+json on the EHR creates and the EHR\_STATUS update. The release defines no EHR\_STATUS form in the Simplified Structured JSON format (upstream report \#352), so the accepting arm has no document. Upstream report [#352](https://github.com/Vernum-Projecten/Veredictum/issues/352): ITS-REST declares the Simplified Formats for EHR\_STATUS, FOLDER and the PARTY kinds, which the Simplified Formats specification never maps (open).

</details>

#### EHR\_STATUS

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `ehr-status-xml-read` | Optional (MAY) | `ehr-status-xml-supported` | 7 | 0 | 0 | 0 | 7 |
| `ehr-status-xml-write` | Optional (MAY) | `ehr-status-xml-write-accepted` | 1 | 0 | 0 | 0 | 1 |
| `ehr-status-flat-read` | `@SpecDiverge` | `ehr-status-flat-unsupported` | 3 | 0 | 0 | 0 | 0 |
| `ehr-status-flat-write` | `@SpecDiverge` | `ehr-status-flat-write-refused` | 1 | 0 | 0 | 0 | 0 |
| `ehr-status-structured-read` | `@SpecDiverge` | `ehr-status-structured-unsupported` | 3 | 0 | 0 | 0 | 0 |
| `ehr-status-structured-write` | `@SpecDiverge` | `ehr-status-structured-write-refused` | 1 | 0 | 0 | 0 | 0 |

<details>
<summary>What these options decide</summary>

- **`ehr-status-xml-read`** (Canonical XML): Whether the service serves the EHR\_STATUS resource family (the EHR\_STATUS, its VERSIONED\_EHR\_STATUS, their versions and the revision history) in canonical XML when a client asks for it with Accept application/xml.
- **`ehr-status-xml-write`** (Canonical XML): Whether the service processes an EHR\_STATUS sent as canonical XML on the EHR\_STATUS update. No published document schema reaches EHR\_STATUS: the global \`items\` element (LOCATABLE) of documents/Structure.xsd does not include Ehr.xsd, which only the 2.0.0 lineage publishes, and no released sentence names the root a service accepts (upstream report \#759). The accepting arm sends an \`items\` root typed EHR\_STATUS in that lineage's namespace, so its workflow rests on that report.
- **`ehr-status-flat-read`** (Simplified Formats and Web Template): Whether the service serves an EHR\_STATUS, on a read and as the representation of an update, when a client asks for it with Accept application/openehr.wt.flat+json. The release defines no EHR\_STATUS form in the Simplified Flat JSON format (upstream report \#352), so the serving arm has no document. Upstream report [#352](https://github.com/Vernum-Projecten/Veredictum/issues/352): ITS-REST declares the Simplified Formats for EHR\_STATUS, FOLDER and the PARTY kinds, which the Simplified Formats specification never maps (open).
- **`ehr-status-flat-write`** (Simplified Formats and Web Template): Whether the service processes an EHR\_STATUS sent with Content-Type application/openehr.wt.flat+json on the EHR creates and the EHR\_STATUS update. The release defines no EHR\_STATUS form in the Simplified Flat JSON format (upstream report \#352), so the accepting arm has no document. Upstream report [#352](https://github.com/Vernum-Projecten/Veredictum/issues/352): ITS-REST declares the Simplified Formats for EHR\_STATUS, FOLDER and the PARTY kinds, which the Simplified Formats specification never maps (open).
- **`ehr-status-structured-read`** (Simplified Formats and Web Template): Whether the service serves an EHR\_STATUS, on a read and as the representation of an update, when a client asks for it with Accept application/openehr.wt.structured+json. The release defines no EHR\_STATUS form in the Simplified Structured JSON format (upstream report \#352), so the serving arm has no document. Upstream report [#352](https://github.com/Vernum-Projecten/Veredictum/issues/352): ITS-REST declares the Simplified Formats for EHR\_STATUS, FOLDER and the PARTY kinds, which the Simplified Formats specification never maps (open).
- **`ehr-status-structured-write`** (Simplified Formats and Web Template): Whether the service processes an EHR\_STATUS sent with Content-Type application/openehr.wt.structured+json on the EHR creates and the EHR\_STATUS update. The release defines no EHR\_STATUS form in the Simplified Structured JSON format (upstream report \#352), so the accepting arm has no document. Upstream report [#352](https://github.com/Vernum-Projecten/Veredictum/issues/352): ITS-REST declares the Simplified Formats for EHR\_STATUS, FOLDER and the PARTY kinds, which the Simplified Formats specification never maps (open).

</details>

#### COMPOSITION

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `composition-flat-read` | Recommended (SHOULD) | `composition-flat-supported` | 5 | 0 | 0 | 0 | 6 |
| `composition-flat-write` | Recommended (SHOULD) | `composition-flat-write-accepted` | 2 | 0 | 0 | 0 | 3 |
| `composition-structured-read` | Recommended (SHOULD) | `composition-structured-supported` | 5 | 0 | 0 | 0 | 6 |
| `composition-structured-write` | Recommended (SHOULD) | `composition-structured-write-accepted` | 2 | 0 | 0 | 0 | 3 |
| `composition-xml-read` | Optional (MAY) | `composition-xml-supported` | 9 | 0 | 0 | 0 | 9 |
| `composition-xml-write` | Optional (MAY) | `composition-xml-write-accepted` | 3 | 0 | 0 | 0 | 2 |

<details>
<summary>What these options decide</summary>

- **`composition-flat-read`** (Simplified Formats and Web Template): Whether the service serves a COMPOSITION in the Simplified Flat JSON format when a client asks for it with Accept. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`composition-flat-write`** (Simplified Formats and Web Template): Whether the service processes a COMPOSITION sent in the Simplified Flat JSON format on the COMPOSITION create and update. The release recommends the Simplified Formats, so a service that does not process this one answers 415.
- **`composition-structured-read`** (Simplified Formats and Web Template): Whether the service serves a COMPOSITION in the Simplified Structured JSON format when a client asks for it with Accept. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`composition-structured-write`** (Simplified Formats and Web Template): Whether the service processes a COMPOSITION sent in the Simplified Structured JSON format on the COMPOSITION create and update. The release recommends the Simplified Formats, so a service that does not process this one answers 415.
- **`composition-xml-read`** (Canonical XML): Whether the service serves the COMPOSITION resource family (the COMPOSITION, its VERSIONED\_COMPOSITION, their versions and the revision history) in canonical XML when a client asks for it with Accept application/xml.
- **`composition-xml-write`** (Canonical XML): Whether the service processes a COMPOSITION sent as canonical XML on the COMPOSITION create and update, an instance of the composition element the published ITS-XML schema declares.

</details>

#### DIRECTORY

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `directory-xml-read` | Optional (MAY) | `directory-xml-supported` | 4 | 0 | 0 | 0 | 4 |
| `directory-xml-write` | Optional (MAY) | `directory-xml-write-accepted` | 2 | 0 | 0 | 0 | 2 |

<details>
<summary>What these options decide</summary>

- **`directory-xml-read`** (Canonical XML): Whether the service serves the directory FOLDER in canonical XML when a client asks for it with Accept application/xml: on a read, latest and by version\_uid, and in the answer of a create and of an update.
- **`directory-xml-write`** (Canonical XML): Whether the service processes a directory FOLDER sent as canonical XML on the directory create and update. The global \`items\` element (LOCATABLE) of documents/Structure.xsd gives a FOLDER a conforming root, but no released sentence requires a service to accept that root (upstream report \#759), so the accepting arm's workflows rest on that report.

</details>

#### CONTRIBUTION

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `contribution-xml-read` | Optional (MAY) | `contribution-xml-unsupported` | 2 | 0 | 0 | 0 | 2 |
| `contribution-xml-write` | Optional (MAY) | `contribution-xml-write-refused` | 1 | 0 | 0 | 0 | 0 |

<details>
<summary>What these options decide</summary>

- **`contribution-xml-read`** (Canonical XML): Whether the service serves a CONTRIBUTION in canonical XML when a client asks for it with Accept application/xml, on a read and in the answer of a create.
- **`contribution-xml-write`** (Canonical XML): Whether the service processes a CONTRIBUTION sent as canonical XML on the CONTRIBUTION create. No published XSD declares an element or a type for the commit envelope (upstream report \#759), so the accepting arm has no document yet.

</details>

### Definitions

#### Templates

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `template-example-flat-read` | Recommended (SHOULD) | not selected |  | | | |  |
| `template-example-structured-read` | Recommended (SHOULD) | not selected |  | | | |  |
| `template-web-template` | Recommended (SHOULD) | `template-web-template-supported` | 2 | 0 | 0 | 0 | 3 |
| `template-example-xml-read` | Optional (MAY) | not selected |  | | | |  |

<details>
<summary>What these options decide</summary>

- **`template-example-flat-read`** (Simplified Formats and Web Template): Whether the service serves the example data instance of a stored ADL 1.4 or ADL 2 operational template in the Simplified Flat JSON format when a client asks for it with Accept. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`template-example-structured-read`** (Simplified Formats and Web Template): Whether the service serves the example data instance of a stored ADL 1.4 or ADL 2 operational template in the Simplified Structured JSON format when a client asks for it with Accept. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`template-web-template`** (Simplified Formats and Web Template): Whether the service serves an ADL 1.4 operational template in the Web Template format when a client asks for it with Accept, on the template read and as the representation of an upload. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`template-example-xml-read`** (Canonical XML): Whether the service serves the example data instance of a stored ADL 1.4 or ADL 2 operational template in canonical XML when a client asks for it with Accept application/xml.

</details>

### Query

#### AQL and stored queries

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `aql-terminology` | Optional (MAY) | `aql-terminology-supported` | 1 | 0 | 0 | 0 | 0 |
| `query-ehr-scope` | Optional (MAY) | `query-ehr-scope-filters` | 6 | 0 | 0 | 0 | 0 |
| TRIAL text | BASE on TRIAL text | | 7 | 0 | 0 | 0 | 0 |

<details>
<summary>What these options decide</summary>

- **`aql-terminology`** (Behaviour the release leaves open): Whether the service resolves a TERMINOLOGY() call against an external terminology server. Which servers a service reaches is its own configuration, so a service that reaches none declares the other arm, which has no document.
- **`query-ehr-scope`** (Behaviour the release leaves open): Whether the service restricts a query to the EHR the ehr\_id query parameter or the openehr-ehr-id request header names. The release lets the backend use the scope for routing and optimisation, so a service that treats it as a hint declares the other arm, which has no document.
- **TRIAL text**: BASE cases whose cited text the release marks TRIAL. They are read with the options and never count in the BASE tally.

</details>

### Demographic

#### Demographic

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `demographic-contribution-xml-read` | Optional (MAY) | `demographic-contribution-xml-unsupported` | 2 | 0 | 0 | 0 | 2 |
| `demographic-contribution-xml-write` | Optional (MAY) | `demographic-contribution-xml-write-refused` | 1 | 0 | 0 | 0 | 0 |
| `party-xml-read` | Optional (MAY) | `party-xml-supported` | 15 | 0 | 0 | 0 | 15 |
| `party-xml-write` | Optional (MAY) | `party-xml-write-accepted` | 10 | 0 | 0 | 0 | 10 |
| `versioned-party-xml-read` | Optional (MAY) | `versioned-party-xml-unsupported` | 4 | 0 | 0 | 0 | 4 |

<details>
<summary>What these options decide</summary>

- **`demographic-contribution-xml-read`** (Canonical XML): Whether the service serves a CONTRIBUTION of the Demographic API in canonical XML when a client asks for it with Accept application/xml, on the read and as the answer of the commit.
- **`demographic-contribution-xml-write`** (Canonical XML): Whether the service processes a CONTRIBUTION sent as canonical XML on the commit of the Demographic API. No published XSD declares a document element or a type for the commit envelope (upstream report \#759), so the accepting arm has no document yet.
- **`party-xml-read`** (Canonical XML): Whether the service serves each of the five PARTY kinds of the Demographic API (PERSON, AGENT, GROUP, ORGANISATION and ROLE) in canonical XML when a client asks for it with Accept application/xml, on a read and as the answer of a create or update.
- **`party-xml-write`** (Canonical XML): Whether the service processes a PARTY sent as canonical XML on the create and update of each PARTY kind. The one published global element of a type a PARTY extends is \`items\` (LOCATABLE) in documents/Structure.xsd, and that schema does not include Demographic.xsd, where the PARTY types are defined, so no published document schema reaches the PARTY types (upstream report \#759). The accepting arm sends an \`items\` root typed with each PARTY kind in the 2.0.0 lineage's namespace, so its workflows rest on that report.
- **`versioned-party-xml-read`** (Canonical XML): Whether the service serves the VERSIONED\_PARTY family of the Demographic API (the VERSIONED\_PARTY, its versions and its revision history) in canonical XML when a client asks for it with Accept application/xml.

</details>

### Content validation

#### Content validation

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `terminology-binding-unresolvable` | Optional (MAY) | `terminology-binding-unresolvable-accepted` | 1 | 0 | 0 | 0 | 0 |
| `content-unmatched-member` | `@SpecDiverge` | `content-unmatched-member-refused` | 2 | 0 | 0 | 0 | 1 |

<details>
<summary>What these options decide</summary>

- **`terminology-binding-unresolvable`** (Behaviour the release leaves open): What the service does with a committed coded value whose archetype constraint binding it cannot resolve, because the terminology query server is unreachable or does not know the value set: refuse the commit, or accept it. Which terminology servers a service reaches is its own configuration. The accepted arm carries the accepted twin of the DV\_CODED\_TEXT external-term refusals; the refused arm has no document, since a refusal of a complete term proves no acceptance.
- **`content-unmatched-member`** (Behaviour the release leaves open): Whether the service refuses a committed member of a container attribute that matches none of the attribute's object constraints. The released AM states the alternative-matching rule for single-valued attributes only, so both postures stay open until upstream report \#307 is settled. Upstream report [#307](https://github.com/Vernum-Projecten/Veredictum/issues/307): AM never states whether a data node matching no constraint node is invalid (open).

</details>

### Simplified formats

#### Simplified formats

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `composition-flat-read` | Recommended (SHOULD) | `composition-flat-supported` | 4 | 0 | 0 | 0 | 0 |
| `composition-flat-write` | Recommended (SHOULD) | `composition-flat-write-accepted` | 109 | 0 | 0 | 0 | 0 |
| `composition-structured-read` | Recommended (SHOULD) | `composition-structured-supported` | 2 | 0 | 0 | 0 | 0 |
| `composition-structured-write` | Recommended (SHOULD) | `composition-structured-write-accepted` | 4 | 0 | 0 | 0 | 0 |
| `contribution-flat-read` | Recommended (SHOULD) | `contribution-flat-supported` | 2 | 0 | 0 | 0 | 3 |
| `contribution-flat-write` | Recommended (SHOULD) | `contribution-flat-write-accepted` | 1 | 0 | 0 | 0 | 2 |
| `contribution-structured-read` | Recommended (SHOULD) | `contribution-structured-supported` | 2 | 0 | 0 | 0 | 3 |
| `contribution-structured-write` | Recommended (SHOULD) | `contribution-structured-write-accepted` | 1 | 0 | 0 | 0 | 2 |
| `simplified-deprecated-read` | Optional (MAY) | `simplified-deprecated-unsupported` | 2 | 0 | 0 | 0 | 2 |
| `simplified-deprecated-write` | Optional (MAY) | `simplified-deprecated-write-refused` | 2 | 0 | 0 | 0 | 2 |
| `simplified-legacy-read` | Optional (MAY) | `simplified-legacy-unsupported` | 1 | 0 | 0 | 0 | 0 |
| `simplified-legacy-write` | Optional (MAY) | `simplified-legacy-write-refused` | 1 | 0 | 0 | 0 | 0 |

<details>
<summary>What these options decide</summary>

- **`composition-flat-read`** (Simplified Formats and Web Template): Whether the service serves a COMPOSITION in the Simplified Flat JSON format when a client asks for it with Accept. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`composition-flat-write`** (Simplified Formats and Web Template): Whether the service processes a COMPOSITION sent in the Simplified Flat JSON format on the COMPOSITION create and update. The release recommends the Simplified Formats, so a service that does not process this one answers 415.
- **`composition-structured-read`** (Simplified Formats and Web Template): Whether the service serves a COMPOSITION in the Simplified Structured JSON format when a client asks for it with Accept. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`composition-structured-write`** (Simplified Formats and Web Template): Whether the service processes a COMPOSITION sent in the Simplified Structured JSON format on the COMPOSITION create and update. The release recommends the Simplified Formats, so a service that does not process this one answers 415.
- **`contribution-flat-read`** (Simplified Formats and Web Template): Whether the service serves a CONTRIBUTION when a client asks for it with Accept application/openehr.wt.flat+json, on a read and in the answer of a create. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`contribution-flat-write`** (Simplified Formats and Web Template): Whether the service processes a CONTRIBUTION sent with Content-Type application/openehr.wt.flat+json, whose canonical envelope carries its members' data in the Simplified Flat JSON format. The release recommends the Simplified Formats, so a service that does not process this one answers 415.
- **`contribution-structured-read`** (Simplified Formats and Web Template): Whether the service serves a CONTRIBUTION when a client asks for it with Accept application/openehr.wt.structured+json, on a read and in the answer of a create. The release recommends the Simplified Formats, so a service that does not serve this one answers 406.
- **`contribution-structured-write`** (Simplified Formats and Web Template): Whether the service processes a CONTRIBUTION sent with Content-Type application/openehr.wt.structured+json, whose canonical envelope carries its members' data in the Simplified Structured JSON format. The release recommends the Simplified Formats, so a service that does not process this one answers 415.
- **`simplified-deprecated-read`** (Simplified Formats and Web Template): Whether the service still serves a template example when a client asks for it with the deprecated media types application/openehr.wt.flat.schema+json and application/openehr.wt.structured.schema+json. A service that has dropped them answers 406.
- **`simplified-deprecated-write`** (Simplified Formats and Web Template): Whether the service still processes a COMPOSITION sent with the deprecated media types application/openehr.wt.flat.schema+json and application/openehr.wt.structured.schema+json. The release deprecates them in favour of their current equivalents, so a service may have dropped them and then answers 415.
- **`simplified-legacy-read`** (Simplified Formats and Web Template): Whether the service serves a template example when a client asks for it with the legacy ECISFLAT media type application/openehr.nc.flat+json. The release names the format and does not define it, so the serving arm has no document; a service that does not serve it answers 406.
- **`simplified-legacy-write`** (Simplified Formats and Web Template): Whether the service processes a COMPOSITION sent with the legacy ECISFLAT media type application/openehr.nc.flat+json. The release names the format and does not define it, so the accepting arm has no document; a service that does not process it answers 415.

</details>

### Admin

#### Admin

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `admin-ehr-delete-all` | Optional (MAY) | `admin-delete-all-enabled` | 7 | 0 | 0 | 0 | 3 |
| `admin-ehr-delete-processing` | Optional (MAY) | `admin-delete-synchronous` | 11 | 0 | 0 | 0 | 3 |

<details>
<summary>What these options decide</summary>

- **`admin-ehr-delete-all`** (Behaviour the release leaves open): Whether the service serves the bulk EHR delete of the Admin API or has it disabled and answers 405.
- **`admin-ehr-delete-processing`** (Behaviour the release leaves open): Whether the service processes an EHR delete of the Admin API synchronously, answering 204 once the EHR is physically deleted, or asynchronously, answering 202 once the delete is accepted. The Admin API is in the DEVELOPMENT state in Release-1.1.0; a service that does not serve it selects neither arm.

</details>

### Security and privacy

#### Security

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `authentication` | Recommended (SHOULD) | `authentication-required` | 5 | 0 | 0 | 0 | 1 |
| `version-signing` | Optional (MAY) | `version-signed` | 4 | 0 | 0 | 0 | 0 |

<details>
<summary>What these options decide</summary>

- **`authentication`** (Behaviour the release leaves open): Whether the service requires authentication. The release recommends an HTTP authentication framework and mandates no scheme, so a service that requires none has no refusal to test.
- **`version-signing`** (Behaviour the release leaves open): Whether the service makes a digital signature of each VERSION at committal and stores it in ORIGINAL\_VERSION.signature. RM common makes the signature optional and leaves its serialisation to be determined, so the signing arm judges the presence of the signature only, and a service that signs nothing has no document.

</details>

### Options no document tests yet

| Option | Kind | Selected arm | Passed | Failed | Warning | Errored | Skipped |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| `xml-namespace` | `@SpecDiverge` | `xml-namespace-v2`, nothing to run | 0 | 0 | 0 | 0 | 0 |
| `aql-nonnumeric-aggregate` | `@SpecDiverge` | `aql-nonnumeric-aggregate-refused`, nothing to run | 0 | 0 | 0 | 0 | 0 |
| `aql-undefined-containment` | `@SpecDiverge` | `aql-undefined-containment-refused`, nothing to run | 0 | 0 | 0 | 0 | 0 |

<details>
<summary>What these options decide</summary>

- **`xml-namespace`** (Canonical XML): Which published canonical XML lineage the service serves: the Release-1.0.2 schemas in namespace http://schemas.openehr.org/v1, or the Release-2.0.0 schemas in http://schemas.openehr.org/v2. The release requires conformance to "the published XSDs" and names neither, so neither arm has a document yet. Upstream report [#759](https://github.com/Vernum-Projecten/Veredictum/issues/759): ITS-REST: the Simplified Formats and canonical XML chapters require what they leave undefined (open).
- **`aql-nonnumeric-aggregate`** (Behaviour the release leaves open): Whether the service refuses a SUM or AVG over an input type outside the function's declared types, or executes it. AQL names the types and assigns no outcome to another, so both arms are open until upstream report \#229 is settled, and neither arm has a document yet. Upstream report [#229](https://github.com/Vernum-Projecten/Veredictum/issues/229): AQL leaves SUM and AVG over an input type outside their declared set unassigned (closed, standing record).
- **`aql-undefined-containment`** (Behaviour the release leaves open): Whether the service refuses a query whose CONTAINS names a pair of classes the RM defines no containment for, or executes it to an empty result. AQL assigns neither, so both arms are open until upstream report \#161 is settled, and neither arm has a document yet. Upstream report [#161](https://github.com/Vernum-Projecten/Veredictum/issues/161): AQL never assigns a behaviour to a containment pair the RM defines no relationship for (confirmed).

</details>

## Skipped rows

A skipped row was not run, for the reason given. It never counts for or against the server.

| Reason | Citation | Rows |
| --- | --- | ---: |
| option-not-selected | specs/openehr/AM/docs/ADL1.4/master05-cadl.adoc, Single-valued Attributes | 1 |
| option-not-selected | specs/openehr/ITS-REST/specifications/docs/overview/Requests\_and\_responses.md, Authentication and authorization | 1 |
| option-not-selected | specs/openehr/ITS-REST/specifications/docs/overview/Resources.md, Data representation | 80 |
| option-not-selected | specs/openehr/ITS-REST/specifications/docs/overview/Resources.md, Simplified Formats | 14 |
| option-not-selected | specs/openehr/ITS-REST/specifications/docs/overview/Resources.md, XML Format | 16 |
| option-not-selected | specs/openehr/ITS-REST/specifications/operations/admin\_ehr\_delete.yaml, description | 3 |
| option-not-selected | specs/openehr/ITS-REST/specifications/operations/admin\_ehr\_delete\_all.yaml, description | 1 |

<details>
<summary>Every skipped row (116)</summary>

| Row | Reason |
| --- | --- |
| `admin_ehr_delete_all_async/delete_all-accepted row 0` | option-not-selected |
| `admin_ehr_delete_all_disabled/delete_all-disabled row 0` | option-not-selected |
| `admin_ehr_delete_async/delete_ehr-accepted row 0` | option-not-selected |
| `composition_read_flat_not_acceptable/get_composition-flat_not_acceptable row 0` | option-not-selected |
| `composition_read_flat_not_acceptable/get_composition-latest-flat_not_acceptable row 0` | option-not-selected |
| `composition_read_flat_not_acceptable/get_composition-at_time-flat_not_acceptable row 0` | option-not-selected |
| `composition_read_flat_not_acceptable/get_composition-flat_warning row 0` | option-not-selected |
| `composition_read_structured_not_acceptable/get_composition-structured_not_acceptable row 0` | option-not-selected |
| `composition_read_structured_not_acceptable/get_composition-latest-structured_not_acceptable row 0` | option-not-selected |
| `composition_read_structured_not_acceptable/get_composition-at_time-structured_not_acceptable row 0` | option-not-selected |
| `composition_read_structured_not_acceptable/get_composition-structured_warning row 0` | option-not-selected |
| `composition_read_xml_not_acceptable/get_composition-xml_not_acceptable row 0` | option-not-selected |
| `composition_read_xml_not_acceptable/get_composition-latest-xml_not_acceptable row 0` | option-not-selected |
| `composition_read_xml_not_acceptable/get_composition-at_time-xml_not_acceptable row 0` | option-not-selected |
| `composition_update_xml_write_refused/update_composition-xml_unsupported_media row 0` | option-not-selected |
| `composition_versioned_read_xml_not_acceptable/get_versioned_composition-xml_not_acceptable row 0` | option-not-selected |
| `composition_versioned_read_xml_not_acceptable/get_versioned_composition_version-xml_not_acceptable row 0` | option-not-selected |
| `composition_versioned_read_xml_not_acceptable/get_versioned_composition_version-by_id-xml_not_acceptable row 0` | option-not-selected |
| `composition_versioned_read_xml_not_acceptable/get_versioned_composition-revision_history-xml_not_acceptable row 0` | option-not-selected |
| `composition_write_flat_not_acceptable/create_composition-accept_flat row 0` | option-not-selected |
| `composition_write_flat_not_acceptable/update_composition-accept_flat row 0` | option-not-selected |
| `composition_write_flat_unsupported/create_composition-flat_unsupported_media row 0` | option-not-selected |
| `composition_write_flat_unsupported/update_composition-flat_unsupported_media row 0` | option-not-selected |
| `composition_write_flat_unsupported/create_composition-flat_warning row 0` | option-not-selected |
| `composition_write_structured_not_acceptable/create_composition-accept_structured row 0` | option-not-selected |
| `composition_write_structured_not_acceptable/update_composition-accept_structured row 0` | option-not-selected |
| `composition_write_structured_unsupported/create_composition-structured_unsupported_media row 0` | option-not-selected |
| `composition_write_structured_unsupported/update_composition-structured_unsupported_media row 0` | option-not-selected |
| `composition_write_structured_unsupported/create_composition-structured_warning row 0` | option-not-selected |
| `composition_write_xml_not_acceptable/create_composition-accept_xml row 0` | option-not-selected |
| `composition_write_xml_not_acceptable/update_composition-accept_xml row 0` | option-not-selected |
| `composition_xml_write_refused/create_composition-xml_unsupported_media row 0` | option-not-selected |
| `content_item_unmatched_accepted/item-unmatched_member_accepted-accepted row 0` | option-not-selected |
| `contribution_read_xml/get_contribution-xml row 0` | option-not-selected |
| `contribution_write_xml_accept/commit_contribution-accept_xml row 0` | option-not-selected |
| `template_adl14_web_template_not_acceptable/upload_template-web_template_not_acceptable row 0` | option-not-selected |
| `template_adl14_web_template_not_acceptable/get_template-web_template_not_acceptable row 0` | option-not-selected |
| `template_adl14_web_template_not_acceptable/get_template-web_template_warning row 0` | option-not-selected |
| `template_example_flat/get_template_adl14_example-flat row 0` | option-not-selected |
| `template_example_flat/get_template_adl2_example-flat row 0` | option-not-selected |
| `template_example_flat_not_acceptable/get_template_adl14_example-flat_not_acceptable row 0` | option-not-selected |
| `template_example_flat_not_acceptable/get_template_adl2_example-flat_not_acceptable row 0` | option-not-selected |
| `template_example_structured/get_template_adl14_example-structured row 0` | option-not-selected |
| `template_example_structured/get_template_adl2_example-structured row 0` | option-not-selected |
| `template_example_structured_not_acceptable/get_template_adl14_example-structured_not_acceptable row 0` | option-not-selected |
| `template_example_structured_not_acceptable/get_template_adl2_example-structured_not_acceptable row 0` | option-not-selected |
| `template_example_xml/get_template_adl14_example-xml row 0` | option-not-selected |
| `template_example_xml/get_template_adl2_example-xml row 0` | option-not-selected |
| `template_example_xml_not_acceptable/get_template_adl14_example-xml_not_acceptable row 0` | option-not-selected |
| `template_example_xml_not_acceptable/get_template_adl2_example-xml_not_acceptable row 0` | option-not-selected |
| `demographic_contribution_read_xml/commit_demographic_contribution-accept_xml row 0` | option-not-selected |
| `demographic_contribution_read_xml/get_demographic_contribution-accept_xml row 0` | option-not-selected |
| `party_read_xml_not_acceptable/get_person-xml_not_acceptable row 0` | option-not-selected |
| `party_read_xml_not_acceptable/get_agent-xml_not_acceptable row 0` | option-not-selected |
| `party_read_xml_not_acceptable/get_group-xml_not_acceptable row 0` | option-not-selected |
| `party_read_xml_not_acceptable/get_organisation-xml_not_acceptable row 0` | option-not-selected |
| `party_read_xml_not_acceptable/get_role-xml_not_acceptable row 0` | option-not-selected |
| `party_write_xml_not_acceptable/create_person-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/update_person-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/create_agent-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/update_agent-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/create_group-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/update_group-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/create_organisation-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/update_organisation-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/create_role-accept_xml row 0` | option-not-selected |
| `party_write_xml_not_acceptable/update_role-accept_xml row 0` | option-not-selected |
| `party_xml_create_refused/create_person-xml_refused row 0` | option-not-selected |
| `party_xml_create_refused/create_agent-xml_refused row 0` | option-not-selected |
| `party_xml_create_refused/create_group-xml_refused row 0` | option-not-selected |
| `party_xml_create_refused/create_organisation-xml_refused row 0` | option-not-selected |
| `party_xml_write_refused/update_person-xml_refused row 0` | option-not-selected |
| `party_xml_write_refused/update_agent-xml_refused row 0` | option-not-selected |
| `party_xml_write_refused/update_group-xml_refused row 0` | option-not-selected |
| `party_xml_write_refused/update_organisation-xml_refused row 0` | option-not-selected |
| `party_xml_write_refused/create_role-xml_refused row 0` | option-not-selected |
| `party_xml_write_refused/update_role-xml_refused row 0` | option-not-selected |
| `versioned_party_read_xml/get_versioned_party-xml row 0` | option-not-selected |
| `versioned_party_read_xml/get_versioned_party_version-xml row 0` | option-not-selected |
| `versioned_party_read_xml/get_versioned_party_version_by_id-xml row 0` | option-not-selected |
| `versioned_party_read_xml/get_versioned_party_revision_history-xml row 0` | option-not-selected |
| `directory_read_xml_not_acceptable/get_directory-xml_not_acceptable row 0` | option-not-selected |
| `directory_read_xml_not_acceptable/get_directory_version-xml_not_acceptable row 0` | option-not-selected |
| `directory_write_xml_not_acceptable/create_directory-accept_xml row 0` | option-not-selected |
| `directory_write_xml_not_acceptable/update_directory-accept_xml row 0` | option-not-selected |
| `directory_xml_write_refused/create_directory-xml_refused row 0` | option-not-selected |
| `directory_xml_write_refused/update_directory-xml_refused row 0` | option-not-selected |
| `ehr_xml_not_acceptable/get_ehr-xml_not_acceptable row 0` | option-not-selected |
| `ehr_xml_not_acceptable/create_ehr-xml_not_acceptable row 0` | option-not-selected |
| `ehr_xml_not_acceptable/create_ehr-xml_not_acceptable-with_id row 0` | option-not-selected |
| `ehr_xml_not_acceptable_json_setup/get_ehr-by_subject-xml_not_acceptable row 0` | option-not-selected |
| `ehr_xml_write_refused/create_ehr-xml_unsupported_media row 0` | option-not-selected |
| `ehr_status_xml_not_acceptable/get_ehr_status-xml_not_acceptable row 0` | option-not-selected |
| `ehr_status_xml_not_acceptable/get_versioned_ehr_status-xml_not_acceptable row 0` | option-not-selected |
| `ehr_status_xml_not_acceptable/get_versioned_ehr_status_version-xml_not_acceptable row 0` | option-not-selected |
| `ehr_status_xml_not_acceptable/get_versioned_ehr_status-revision_history-xml_not_acceptable row 0` | option-not-selected |
| `ehr_status_xml_not_acceptable_json_setup/get_ehr_status-by_version_id-xml_not_acceptable row 0` | option-not-selected |
| `ehr_status_xml_not_acceptable_json_setup/get_versioned_ehr_status_version-by_id-xml_not_acceptable row 0` | option-not-selected |
| `ehr_status_xml_not_acceptable_json_setup/update-xml_not_acceptable row 0` | option-not-selected |
| `ehr_status_xml_write_refused/update-xml_unsupported_media row 0` | option-not-selected |
| `authentication_not_required/ehr_create-invalid_credentials_warning row 0` | option-not-selected |
| `contribution_read_flat_not_acceptable/get_contribution-flat_not_acceptable row 0` | option-not-selected |
| `contribution_read_flat_not_acceptable/get_contribution-flat_warning row 0` | option-not-selected |
| `contribution_read_structured_not_acceptable/get_contribution-structured_not_acceptable row 0` | option-not-selected |
| `contribution_read_structured_not_acceptable/get_contribution-structured_warning row 0` | option-not-selected |
| `contribution_write_flat_not_acceptable/commit_contribution-accept_flat row 0` | option-not-selected |
| `contribution_write_flat_unsupported/commit_contribution-flat_unsupported_media row 0` | option-not-selected |
| `contribution_write_flat_unsupported/commit_contribution-flat_warning row 0` | option-not-selected |
| `contribution_write_structured_not_acceptable/commit_contribution-accept_structured row 0` | option-not-selected |
| `contribution_write_structured_unsupported/commit_contribution-structured_unsupported_media row 0` | option-not-selected |
| `contribution_write_structured_unsupported/commit_contribution-structured_warning row 0` | option-not-selected |
| `deprecated_example/get_template_example-flat_deprecated_media row 0` | option-not-selected |
| `deprecated_example/get_template_example-structured_deprecated_media row 0` | option-not-selected |
| `deprecated_write/create_composition-flat_deprecated_media row 0` | option-not-selected |
| `deprecated_write/create_composition-structured_deprecated_media row 0` | option-not-selected |
| `admin_ehr_delete_all_unfiltered_async/delete_all-unfiltered_accepted row 0` | option-not-selected |

</details>

---

Spec divergence: 147 case(s) rest on released text that is unclear or contradictory, and are tagged @SpecDiverge. Until the specification resolves it, Veredictum judges them as the linked upstream reports describe: [#307](https://github.com/Vernum-Projecten/Veredictum/issues/307), [#352](https://github.com/Vernum-Projecten/Veredictum/issues/352), [#353](https://github.com/Vernum-Projecten/Veredictum/issues/353), [#661](https://github.com/Vernum-Projecten/Veredictum/issues/661), [#673](https://github.com/Vernum-Projecten/Veredictum/issues/673), [#721](https://github.com/Vernum-Projecten/Veredictum/issues/721), [#724](https://github.com/Vernum-Projecten/Veredictum/issues/724), [#735](https://github.com/Vernum-Projecten/Veredictum/issues/735), [#750](https://github.com/Vernum-Projecten/Veredictum/issues/750), [#752](https://github.com/Vernum-Projecten/Veredictum/issues/752), [#758](https://github.com/Vernum-Projecten/Veredictum/issues/758), [#759](https://github.com/Vernum-Projecten/Veredictum/issues/759), [#771](https://github.com/Vernum-Projecten/Veredictum/issues/771), [#939](https://github.com/Vernum-Projecten/Veredictum/issues/939), [#945](https://github.com/Vernum-Projecten/Veredictum/issues/945), [#946](https://github.com/Vernum-Projecten/Veredictum/issues/946), [#984](https://github.com/Vernum-Projecten/Veredictum/issues/984).
