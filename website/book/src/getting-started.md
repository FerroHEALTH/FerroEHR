# Getting started

This chapter takes you from nothing to a running server with a template loaded,
a clinical composition stored, and an AQL query returning results in a few
minutes, using Docker Compose. It is the fastest way to see FerroEHR work end to
end and to get a feel for the API before reading the reference chapters.
Everything here uses the built-in development credentials; do not use them
outside local evaluation.

> [!WARNING]
> The steps below enable Basic auth with the throwaway user `ferroehr` /
> `ferroehr`, leave role-based access control **off** (so that one user reaches
> every enabled surface, admin API included), and use a permissive CORS policy.
> This is a development configuration only. See
> [Security](security.md) and the
> [configuration reference](installation/configuration.md) before exposing a
> server.

## 1. Start the stack

You need Docker with the Compose plugin (2.23.1 or newer). Download
`docker-compose.yml` (attached to every
[release](https://github.com/FerroHEALTH/FerroEHR/releases/latest)) into an
empty directory and start it:

```shell
docker compose up
```

This pulls and starts two services: the server (`ferroehr`) on port **8080**,
and a preconfigured **PostgreSQL 18** database. The server runs its schema
migrations automatically on first boot, so the database is ready as soon as it
reports healthy. Nothing else is needed: the server's configuration travels
inside the Compose file, which also ships one Basic-auth user (`ferroehr` /
`ferroehr`, holding both the `ADMIN` and `USER` roles) so the API authenticates
out of the box.

Published ports bind `127.0.0.1` by default, so the stack is reachable from
this machine and not from the network. Three things are optional and stay down
until you ask for them: the [viewer](viewer/index.md)
(`docker compose --profile viewer up`, then <http://localhost:3000>), a
SeaweedFS S3 gateway for multimedia (`--profile s3`), and a ready-made Keycloak
identity provider for bearer-token auth (a second downloadable overlay,
`docker-compose.keycloak.yml`). See [Docker Compose](installation/compose.md)
for all of them.

## 2. Probe the status endpoint

The status endpoint is public and confirms the server is up:

```shell
curl http://localhost:8080/ferroehr/rest/status
```

It answers a small JSON document: `status`, `server_version`,
`openehr_rest_api_version`, a `timestamp`, the `licence` in force and the
declared `deployment` profile. All clinical API routes live under
the base path `/ferroehr/rest/openehr/v1`. Interactive OpenAPI documentation is
served at <http://localhost:8080/ferroehr/rest/swagger-ui>; sign in with the
API credential when the browser asks (the UI is served to authenticated users by
default, see [`server.swagger_ui`](installation/config-server.md#server)). If you have no
server yet, use the hosted sandbox: <https://sandbox.ferroehr.eu> opens the
viewer over a live CDR, and the same server's Swagger UI is at
<https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui>. Both take the public
demo credentials `ferroehr` / `ferroehr`. The sandbox is a shared demo server
that resets every night, so treat anything you write there as temporary and
expect it to slow down under heavy use.

### What is already in the sandbox

The nightly reset reloads a fixed demo dataset, so the server always answers
with something to look at:

- **8 EHRs**, each with a mixed record and its own subject id
  (`sandbox-patient-01` … `-08` in the `ferroehr-sandbox` namespace, so
  `GET /ehr?subject_id=sandbox-patient-01&subject_namespace=ferroehr-sandbox`
  finds one). Two of them carry a second `EHR_STATUS` version: patient 07 is
  not queryable and so never appears in AQL results, patient 08 is not
  modifiable and refuses writes with a `409`.
- **183 compositions** across **17 ADL 1.4 templates**: sixteen from the
  openEHR CKM (vital signs, problem and medicines lists, lab results,
  referrals, an International Patient Summary, several statutory case-report
  forms) and one terminology-binding template whose coded text resolves
  against the terminology server running beside the CDR. Six compositions
  have more than one version, so `LATEST_VERSION` and `ALL_VERSIONS` differ on
  real data.
- **233 ADL 2 archetypes and 5 ADL 2 templates**, browsable under
  `definition/archetype/adl2` and `definition/template/adl2`. Two of the
  templates are ADL 2 source templates whose slots the server flattens against
  that archetype library, and the compositions committed from them came out of
  the server's own `…/example` generator.
- **11 demographic parties** (persons, roles, party relationships) and a
  **FOLDER directory** in every EHR whose items reference real compositions.
- **5 stored AQL queries** under `eu.ferroehr.sandbox`, all of which return
  rows: `composition_index`, `blood_pressure`, `case_reports`,
  `composition_versions`, and `patient_record` (takes an `$ehr_id` parameter).
  Run one with
  `GET /query/eu.ferroehr.sandbox::composition_index`, or open the viewer's
  query screen.

There are also three always-on, unauthenticated health endpoints: `/health`,
`/health/liveness` and `/health/readiness`; the last one reports each
dependency it checked. See [Operations → Health probes](operations.md#health-probes).

## 3. Create an EHR

An **EHR** is the container for one subject's records. Create one with a `POST`
(no body needed):

```shell
curl -u ferroehr:ferroehr -X POST -i \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr
```

The `-i` flag shows the response headers. On success you get `201 Created`; the
new EHR's identifier is in the `ETag` header (as the weak form
`W/"<ehr_id>"`), and `Location` points at the created resource. Copy the UUID;
the examples below refer to it as `EHR_ID`.

By default the response body is empty. Add `-H 'Prefer: return=representation'`
to have the server return the full `EHR` object instead, or
`-H 'Prefer: return=identifier'` for just the `uid`; either way
`Preference-Applied` echoes what the server honoured.

## 4. Upload a template

Before you can store a composition, the server needs the **Operational Template
(OPT 1.4)** that the composition conforms to. Templates are XML documents;
upload one with `Content-Type: application/xml`:

```shell
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/xml' \
  --data-binary @my-template.opt \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4
```

A successful upload returns `201 Created`. If you do not have a template to
hand, the openEHR community publishes example OPTs (for instance the
`Vital Signs` templates used in the openEHR training material), and the
international [Clinical Knowledge Manager](https://ckm.openehr.org/) is the
source for the archetypes they are built from. List what is loaded, and inspect
a template's derived WebTemplate (a JSON description convenient for building
forms), with:

```shell
# List templates
curl -u ferroehr:ferroehr \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4

# Fetch the WebTemplate for one template
curl -u ferroehr:ferroehr \
  -H 'Accept: application/openehr.wt+json' \
  http://localhost:8080/ferroehr/rest/openehr/v1/definition/template/adl1.4/my_template_id
```

See [Templates & validation](templates-validation.md) for the full template
lifecycle and the WebTemplate/FLAT/STRUCTURED formats.

## 5. Commit a composition

A **composition** is one clinical document, stored inside an EHR and validated
against its template. Post the composition JSON (its `archetype_details` name
the template it belongs to):

```shell
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/json' \
  -H 'Prefer: return=representation' \
  --data-binary @my-composition.json \
  http://localhost:8080/ferroehr/rest/openehr/v1/ehr/$EHR_ID/composition
```

On success you get `201 Created` and (because of `Prefer: return=representation`)
the stored composition in the body, now carrying a server-assigned version
identifier in its `uid`. If the composition does not conform to its template you
get `422 Unprocessable Entity` with the validation errors; a malformed request
gets `400 Bad Request`. The composition walkthrough in
[Resource walkthroughs](using-the-api/resources.md) covers update and delete,
which use the `If-Match` header for optimistic concurrency.

## 6. Query with AQL

Now query across the data with the Archetype Query Language. The simplest query
lists the EHR ids the server holds:

```shell
curl -u ferroehr:ferroehr \
  -H 'Content-Type: application/json' \
  -d '{"q":"SELECT e/ehr_id/value FROM EHR e"}' \
  http://localhost:8080/ferroehr/rest/openehr/v1/query/aql
```

The response is a `RESULT_SET`: a `columns` array describing each selected value
and a `rows` array of result tuples. To pull values out of the compositions you
committed, select by their archetype path, for example every systolic blood
pressure above 140:

```shell
curl -u ferroehr:ferroehr -H 'Content-Type: application/json' -d '{
  "q": "SELECT o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude AS systolic FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2] WHERE o/data[at0001]/events[at0006]/data[at0003]/items[at0004]/value/magnitude > 140"
}' http://localhost:8080/ferroehr/rest/openehr/v1/query/aql
```

[Querying with AQL](querying-aql.md) is the full language guide — parameters,
stored queries, version scope, terminology, pagination, and the supported
feature set.

## 7. Explore the API interactively

Open <http://localhost:8080/ferroehr/rest/swagger-ui> (signing in with the API
credential when the browser asks) to browse and try every endpoint from your
browser. The UI's spec selector carries one entry per API
family: the standardised openEHR groups (EHR, Query, Definition, Demographic,
Admin) and the server's own extensions (status & management, terminology, party
relationships, messaging, event subscriptions, the FHIR connector, SMART
discovery), plus **FerroEHR — Complete surface** last, which
is the whole server in one document. Every entry is filtered from that same
document, which the server generates from its own handlers, so nothing here can
drift from the routes it actually serves. When authentication is enabled the
"Authorize" dialog shows the one scheme the server is configured for (HTTP
Bearer/JWT when OIDC is set up, otherwise HTTP Basic). The hosted sandbox runs
the same UI at <https://sandbox.ferroehr.eu/ferroehr/rest/swagger-ui> if you
want to try it before running anything locally — and
<https://sandbox.ferroehr.eu> itself opens the [viewer](viewer/index.md)
on that server, with the demo credentials `ferroehr` / `ferroehr`.

## Next steps

- [Installation](installation/index.md) — running it for real (Compose,
  Kubernetes, or from source) and the
  [configuration reference](installation/configuration.md).
- [Using the API](using-the-api/index.md) — the per-resource reference with
  headers, status codes, and versioning.
- [Operations](operations.md) — migrations, probes, observability and upgrades,
  plus the [admin & messaging APIs](operations-admin-apis.md).
- [Concepts](concepts/index.md) — the openEHR model and how FerroEHR is built,
  if the terms above were unfamiliar.
