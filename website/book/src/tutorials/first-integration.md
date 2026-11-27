<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# Your first integration

**For:** developers building on the REST API. This tutorial takes you from an
empty server to a stored, updated and queried composition, and shows the two
answers every integration has to handle: a stale version and a missing record.
Every request on this page is run against the server in CI, so the requests
and the statuses they show are the ones the server gives.

<!-- toc -->

## Before you start

Run the quickstart ([Getting started](../getting-started.md)). The requests
below are plain HTTP against `http://localhost:8080`. Send them with any HTTP
client and add your credentials: on the quickstart that is HTTP Basic
`ferroehr` / `ferroehr` (`curl -u ferroehr:ferroehr`). A production deployment
takes an OAuth 2.0 bearer token from your authorization server instead
([SMART App Launch](../smart-app-launch.md), [Enterprise identity
providers](../identity-providers.md)).

Download two files from the repository: the vital signs template
[`vital-signs.opt`](https://github.com/FerroHEALTH/FerroEHR/blob/main/corpus/templates/ckm/vital-signs.opt)
and an example composition for it,
[`vital-signs.example.json`](https://github.com/FerroHEALTH/FerroEHR/blob/main/corpus/templates/ckm/vital-signs.example.json).

Placeholders in braces, such as `{ehr_id}`, stand for a value an earlier
answer gave you.

## 1. Upload the template

A template tells the server what a composition may contain
([Templates & validation](../templates-validation.md)). Send the file as the
body:

<!-- book-request: upload-template expect=201 body-file=corpus/templates/ckm/vital-signs.opt -->
```http
POST /ferroehr/rest/openehr/v1/definition/template/adl1.4
Content-Type: application/xml
```

The server answers `201 Created`. Uploading the same template again answers
`409 Conflict`.

## 2. Create an EHR

<!-- book-request: create-ehr expect=201 capture=ehr_id:/ehr_id/value -->
```http
POST /ferroehr/rest/openehr/v1/ehr
Prefer: return=representation
```

`Prefer: return=representation` asks for the new EHR in the body. Keep its
`ehr_id.value`: every clinical request names it.

## 3. Commit a composition

<!-- book-request: commit expect=201 body-file=corpus/templates/ckm/vital-signs.example.json capture=uid:/uid/value capture=object:/uid/value|object -->
```http
POST /ferroehr/rest/openehr/v1/ehr/{ehr_id}/composition
Content-Type: application/json
Prefer: return=representation
```

The answer carries the composition with its version id in `uid.value`, such
as `8849182c-82ad-4088-a07f-48ead4180515::local.ferroehr::1`: the versioned
object's id, the system that created it and the version number. The `ETag`
header carries the same id.

## 4. Read it back

<!-- book-request: read expect=200 -->
```http
GET /ferroehr/rest/openehr/v1/ehr/{ehr_id}/composition/{uid}
Accept: application/json
```

Ask for `application/xml` to get canonical XML, or a simplified format
([Content negotiation & errors](../using-the-api/content-negotiation.md)).

## 5. Update it

An update names the versioned object in the path and the version you read in
`If-Match`, so two clients cannot overwrite each other unseen:

<!-- book-request: update expect=200 body-file=corpus/templates/ckm/vital-signs.example.json capture=uid2:/uid/value -->
```http
PUT /ferroehr/rest/openehr/v1/ehr/{ehr_id}/composition/{object}
Content-Type: application/json
Prefer: return=representation
If-Match: "{uid}"
```

The answer is version 2. Every earlier version stays readable: an openEHR
repository never overwrites a version.

## 6. Handle a stale version

Send the same update again, still naming version 1:

<!-- book-request: stale-update expect=412 body-file=corpus/templates/ckm/vital-signs.example.json -->
```http
PUT /ferroehr/rest/openehr/v1/ehr/{ehr_id}/composition/{object}
Content-Type: application/json
If-Match: "{uid}"
```

The server answers `412 Precondition Failed`, because version 2 is now the
latest. Read the latest version, apply your change to it, and send the update
with its id.

## 7. Query with AQL

<!-- book-request: query expect=200 json-has=/rows/0 -->
```http
POST /ferroehr/rest/openehr/v1/query/aql
Content-Type: application/json

{
  "q": "SELECT c/uid/value FROM EHR e[ehr_id/value=$ehr_id] CONTAINS COMPOSITION c",
  "query_parameters": { "ehr_id": "{ehr_id}" }
}
```

Each row holds the selected columns of one composition: here the id of its
latest version. [Querying with AQL](../querying-aql.md) covers the language.

## 8. Handle a missing record

<!-- book-request: missing expect=404 header=content-type:application/problem+json -->
```http
GET /ferroehr/rest/openehr/v1/ehr/00000000-0000-4000-8000-000000000000
Accept: application/json
```

An error is an RFC 9457 problem details document. Its `type` names what went
wrong, and [Problem types](../using-the-api/problems.md) lists them all. A
`403` means the access decision refused the caller
([How FerroEHR decides access](../explanation/access-model.md)).

## Next

- [Resource walkthroughs](../using-the-api/resources.md): every resource and
  operation.
- [Content negotiation & errors](../using-the-api/content-negotiation.md):
  formats, `Prefer`, `ETag` and `If-Match` in detail.
