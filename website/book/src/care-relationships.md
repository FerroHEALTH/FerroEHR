<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Care relationships and episodes of care

The European Health Data Space Regulation gives a health professional
access to "the relevant and necessary personal electronic health data of
natural persons under their treatment" (Regulation (EU) 2025/327, Art.
11(1)), through a health professional access service, "a service, supported
by an EHR system, that enables health professionals to access data of
natural persons under their treatment" (Art. 2(2)(i)). The Regulation does
not say how a system knows who is under whose treatment. openEHR answers
that the care delivery environment knows: the evaluation of "care deliverer"
into identities "must be done in each care delivery environment, not in the
EHR" (BASE Architecture Overview, Security, The Problem of Roles).

FerroEHR therefore stores the outcome as a fact, the care relationship, and
decides every access and every query on it. This page explains the model.
The fields, the policy attributes and the refusals are in
[Caller facts: roles and care relationships](operational-policy.md#caller-facts-roles-and-care-relationships)
and [Care relationship facts](operational-policy.md#care-relationship-facts),
and where care sits among the other steps of a decision is on [How FerroEHR
decides access](explanation/access-model.md).

<!-- toc -->

## The caseload

**Your caseload is the patients reachable through your care relationships,
narrowed to your episodes when they name one.**

- **Reachable** means a live care relationship names you: your `PERSON`
  or a `ROLE` you perform (the `direct` path), or a care team or unit you
  are a member of (the `team` path). A relationship is live from its start
  date to its end date, plus the access policy's `relationship_grace`, for a
  provider you act for. A relationship you wrote naming yourself never
  counts, and neither does a self-declared one.
- **Narrowed to your episodes** means that a relationship that names an
  episode of care reaches only the data of that episode, the data placed in
  no episode and the data the access policy's `episode_shared` names. A
  relationship that names no episode reaches the whole record.

The access decision reads your caseload for every named read, export and
write of a patient's data. A query reads it inside its one SQL statement,
so `COUNT`, `LIMIT`, `offset` and `fetch` only ever see data in your
caseload. A query can narrow the caseload further with the `caseload`,
`caseload_roles` and `caseload_episodes` members
([Narrowing a query to your caseload](querying-aql.md#narrowing-a-query-to-your-caseload)),
and a SMART on openEHR session launched in an episode reads that episode
alone ([SMART App Launch](smart-app-launch.md)). A worklist screen built on
the caseload is your application's ([What the CDR leaves to the
application](application-boundary.md)).

## Episodes of care

A patient admitted to cardiology and seen by dermatology has two episodes.
The openEHR specifications name the episode of care ("the current episode of
care, and for some reasonable, limited time afterward", BASE Architecture
Overview, Security Policy) and leave which data belong to it, and who may
read them, open. FerroEHR fills that gap as follows. No openEHR
specification defines these rules: they are FerroEHR's own design, written
as an addition to the draft access control package.

| Part | What FerroEHR does |
| --- | --- |
| The episode | A `DV_IDENTIFIER` the care delivery environment gives the episode, compared by `issuer` and `id`, such as a FHIR `EpisodeOfCare.identifier` or an HL7 v2 visit number. FerroEHR adds no party or RM object for it. An episode is open for a carer while the carer's relationship to it is live. |
| The episode relationship | A care relationship with an `episode` element. Its dates, grace, role, path, provider and cover work as for any care relationship. |
| Placement | A COMPOSITION names its episodes with `episode` elements in `context.other_context`. A composition with none, an `EHR_STATUS`, a `FOLDER` and an `EHR_ACCESS` are placed in no episode. |
| Persistent compositions | A persistent composition, such as the problem list, is never placed: one that names an episode is refused `422`. |
| Declared episodes | A composition placed in an episode that no care relationship of its subject names, live or ended, is refused `422`. Write the relationship first. |
| Coverage | A relationship covers a datum when it names no episode, the datum is unplaced, the datum is in its episode, or `episode_shared` names it. For an access service caller it also covers the EHDS priority categories (Art. 14(1)), since Art. 11(3) makes the access "include at least the priority categories". |
| Refusal | Where care is what would admit you, a named read of a datum no relationship of yours covers is refused `403` with `refused-outside-episode`, at step 7 of the access record. A query, a count and a page leave it out. |
| Access record | The record of an admitted access names the episode of the relationship it rested on, in its `ACCESS_STORED_CARE_GROUND`. A refusal outside your episodes is recorded with its code. |
| Unchanged | The patient, an emergency, a permit directive and a gate-keeper decide as they do without episodes. An episode only narrows what the same relationship without one admits. |
| The patient's choices | A directive, a controller rule, a classification or a delegation names an episode's data with `episodes` in its data scope, so a patient can restrict one episode and nothing else (EHDS Art. 8, "all or parts"). |

Closing an episode is ending its relationships: the carer keeps access for
the policy's `relationship_grace` after the end, as for any relationship.
An access policy can require an episode on the relationships of named roles
and provider types with `episode_required` on a `relationship_validity`
entry.

## What it costs

FerroEHR holds care relationships on every replica beside the grants and
confirms them against the party store with one indexed read per request, so
episodes add no round trip. A caller who holds no relationship scoped to an
episode runs the same SQL statements, byte for byte, as before episodes
existed.

### Statements per request

Measured by the `statement_budget::care` test, each figure the mean over
ten requests, after three warm-up requests, in the hospital scenario of the
test suite. A round trip is one
exchange with PostgreSQL; a nested statement is one a function runs inside
another.

| Request | Round trips | Nested statements | WAL of its rows |
| --- | ---: | ---: | ---: |
| A care-gated read | 2 | 0 | 0 B |
| The same read of a datum in your episode | 2 | 0 | 0 B |
| The same read in a SMART session launched in an episode | 2 | 0 | 0 B |
| A query narrowed to your caseload | 4 | 0 | 0 B |
| The same query under an episode relationship | 4 | 0 | 0 B |
| A care relationship commit by the ADT system | 26 | 2 | about 7.1 kB |

An episode costs no statement: the read and the query under an episode
relationship send exactly the statements of their plain forms.

### Plans at practice scale

Measured by the `care_caseload_plans` test, which holds the plan shapes of
these queries. Three care teams hold caseloads of 100 patients (a small
practice), 1,600 (a ward) and 50,000 (a large practice list). Each query runs
through the service under a policy that requires a care relationship, with
the access decision applied to every row.

{{#include ../generated/care-caseload-bench.md}}

With every relationship of the three teams scoped to one episode the figures
are the same within the run's noise. The check is one probe of the
projection's subject index at every size. A page starts from your own rows
of the caseload index, so its cost follows the size of your caseload and not
the size of the store. A count reads every composition it counts, here
100,000 at the largest caseload, so it grows with the caseload; page your
worklist queries rather than counting a whole practice list.

These are local, single-session figures that show the shape of the work.
Throughput and tail latency under load are measured on the reference host
([Capacity](capacity.md), [Benchmarks](benchmarks.md)).
