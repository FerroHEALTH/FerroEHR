# Records of processing

[GDPR Art. 30](https://eur-lex.europa.eu/eli/reg/2016/679/oj) requires a
controller to keep a record of its processing activities, and Art. 30(2)
requires a shorter one from a processor acting on a controller's behalf. Both
records are the organisation's, not the software's. This page is a template for
the part a FerroEHR deployment contributes: the activities the software
performs by design, with the fields it can answer already filled and the fields
only you can answer marked.

FerroEHR is software rather than a controller or a processor, so nothing here
is a record on anyone's behalf. It is a starting draft that saves you reading
the source to find out what the server does with personal data.

<!-- toc -->

## How to use this

Copy the activity tables below into your own register and fill the three
columns nobody but you can fill: the **purpose** your organisation processes
for, the **lawful basis** it relies on, and the **recipients** it discloses to.
Delete any activity your deployment has switched off. Activities 6 and 7 are
off by default and only apply if you turned them on.

The identity fields Art. 30(1)(a) asks for are yours in every case:

| Field | Value |
|---|---|
| Controller (name, contact) | *your organisation* |
| Joint controllers, if any | *yours* |
| Representative in the Union, if applicable | *yours* |
| Data protection officer | *yours* |
| Processors engaged | *yours*: the PostgreSQL operator, and any object store, message broker, terminology server or identity provider the deployment is configured against |

> [!NOTE]
> One transmission is on by default and is not a processor relationship: the
> [usage report](../usage-report.md) to FerroPULSE. It carries no patient data
> and no staff identities. Cadasto B.V. receives it as controller for its own
> purposes, and its collector stores the IP address each report comes from.
> Where that makes the report personal data (an instance run by one person
> from their own connection, for example), list Cadasto B.V. as a recipient in
> your record, or switch the report off.

## 1. Storing the clinical record

| Art. 30 field | What the software does |
|---|---|
| Purpose | *yours*: the care or research purpose the repository serves |
| Categories of data subject | Patients and other record subjects; the health professionals recorded as composers, performers and participants |
| Categories of personal data | Clinical content as the operational templates define it, in the `ehr` schema and its `cold` archival tier. Special-category health data under Art. 9. The subject appears as an opaque pseudonym once `[privacy] subject_namespaces` is declared |
| Recipients | *yours*: whoever the API is exposed to. The software discloses nothing on its own |
| Third-country transfers | *yours*: wherever the deployment and its backups run |
| Retention | No automatic expiry. Content leaves only by an administrator's deletion; archiving moves it to the cold tier without expiring it |
| Security measures | Least-privilege database roles per pseudonymisation domain; a data-minimisation pass over every write; one instance per organisation; TLS; authentication and layered authorization |

## 2. Versioning and change control

| Art. 30 field | What the software does |
|---|---|
| Purpose | Keeping the clinical record attributable and reconstructible, which openEHR's change-control model requires |
| Categories of data subject | As activity 1, plus the committing user |
| Categories of personal data | One contribution and one audit entry per commit, naming the committer, the change type and the commit instant; every prior version of every object |
| Recipients | *yours* |
| Third-country transfers | *yours* |
| Retention | Indefinite by design. A version is superseded, never overwritten |
| Security measures | Contribution and audit written in the same transaction as the content; a temporal version table rather than in-place update; optional version signing |

## 3. Query

| Art. 30 field | What the software does |
|---|---|
| Purpose | *yours*: the reporting, cohort or operational purpose queries serve |
| Categories of data subject | As activity 1 |
| Categories of personal data | Whatever the query projects out of stored clinical content |
| Recipients | *yours* |
| Third-country transfers | *yours* |
| Retention | Results are not stored. Stored query definitions are, as AQL text |
| Security measures | Queries run on the clinical credential only, so no query can reach the demographic or linkage domain; the planning gate refuses constructs the active specification generation does not define; result caps and statement timeouts bound one query's reach |

> [!NOTE]
> Small-cell suppression applies to the cohort query only. A cohort result
> serving fewer distinct EHRs than `cohort.small_cell_threshold` has its rows
> withheld and is marked suppressed; an ordinary `POST /query/aql` takes no
> threshold, so a query returning one row about one rare condition is served
> like any other.

## 4. Identity and linkage

| Art. 30 field | What the software does |
|---|---|
| Purpose | Holding who the record subjects are, and resolving a person to their record where a caller is entitled to |
| Categories of data subject | Patients, and the parties a record refers to |
| Categories of personal data | Parties in the `demographic` schema with names, addresses and contacts as the template defines them; protected national identifiers sealed in `national_identifier`; the party-to-EHR map in `linkage`, which holds identifiers and a validity period and nothing else |
| Recipients | *yours* |
| Third-country transfers | *yours* |
| Retention | Parties leave only by an administrator's deletion. A linkage mapping leaves only with the EHR it names: a merge or a split closes its period and opens a successor, and erasing the EHR erases the rows naming it |
| Security measures | Three schemas under three roles, revoked from each other in both directions and checked at boot; AES-256-GCM sealing with a keyed HMAC-SHA-256 digest for lookup, under per-domain subkeys; the one crossing runs in the application over two pools and writes an access record |

## 5. Access logging

| Art. 30 field | What the software does |
|---|---|
| Purpose | Recording who reached what, which [EHDS Annex II 3.2](https://eur-lex.europa.eu/eli/reg/2025/327/oj) requires of an EHR system and which [NEN 7513](https://www.nen.nl/nen-7513-2018-nl-245399) specifies the content of for Dutch deployments |
| Categories of data subject | The record subject, and the person who accessed the record |
| Categories of personal data | Per access: the accessing person and organisation, the roles held, the object and its domain, the outcome, the declared purpose of use, the configured legal basis and the time. Refusals are recorded too, unattributed where no principal was established |
| Recipients | *yours*: the local repository, plus any syslog or FHIR-feed Audit Record Repository configured |
| Third-country transfers | *yours* |
| Retention | `[audit.store] retention_days`, `0` meaning forever, reaped hourly otherwise. Where an active identifier rule names a jurisdiction with a registered floor, a shorter value is refused at boot |
| Security measures | Records are hash-chained and verifiable with `audit.verify_audit_chain()`; the runtime role may insert and stamp delivery and nothing else; the `audit` schema sits outside all three pseudonymisation domains; the syslog feed can run over RFC 5425 TLS with a client certificate |

## 6. Change-event publication (off by default)

| Art. 30 field | What the software does |
|---|---|
| Purpose | Telling downstream systems that a commit happened, when `[events]` is enabled |
| Categories of data subject | Record subjects, indirectly |
| Categories of personal data | A PHI-free envelope: the contribution id, the EHR id, the commit instant, and per version the object id, kind, version number, change type and template. No clinical content |
| Recipients | The configured AMQP broker, and whoever consumes from it |
| Third-country transfers | *yours*: wherever the broker runs |
| Retention | Published rows pruned after `[events] retention_days`, seven days by default |
| Security measures | Written in the same transaction as the commit it announces, so there is no event without its commit; optional TLS to the broker; the envelope carries identifiers, so a consumer still has to authenticate to the API to read anything |

## Fields no software can fill

The record is not complete until these are answered, and they are answered by
the organisation:

- The **purpose** of each activity and the **lawful basis** for it, which for
  health data means an Art. 9(2) condition beside the Art. 6 one.
- The **recipients**, including every category of person inside the
  organisation who can reach the API.
- **Transfers to third countries** and the safeguard relied on, which follows
  from where the database, the backups and every configured peer run.
- **Time limits for erasure** per category, which the software will not apply
  for you: it deletes when an administrator asks.
- The **general description of security measures** in Art. 30(1)(g), for which
  the technical half is [the DPIA page](dpia.md) and the
  [threat model](../threat-model.md).

## Next

- [Data protection impact assessment](dpia.md) — the technical description and
  the risk register.
- [Go-live checklist](go-live-checklist.md) — what to verify before real data
  arrives.
- [Shared responsibility](../compliance/shared-responsibility.md) — which
  duties the software can carry.
