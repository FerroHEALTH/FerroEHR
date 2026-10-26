# Secondary use

Research data leaves FerroEHR through FerroBRIDGE, which loads the OMOP Common
Data Model in batches over AQL. FerroEHR holds the record and the marks that
decide what may leave; FerroBRIDGE runs the batch, pseudonymises for research
and maps to OMOP. Every edge between the two is a published specification:
FerroBRIDGE reads through the openEHR REST API, and FerroEHR keeps no outbox
or read model for it.

This page is the contract between the two: what FerroBRIDGE's batch asks, what
FerroEHR answers, and what each side records.

## The batch query

FerroBRIDGE's `etl run` sends one AQL query per batch through
`POST /query/aql` (ITS-REST 1.1.0, `execute_ad_hoc_query`):

```sql
SELECT e/ehr_id/value AS ehr_id, v/uid/value AS version_uid,
       vo/uid/value AS versioned_object_uid, c AS composition
FROM EHR e CONTAINS VERSIONED_OBJECT vo
     CONTAINS VERSION v[LATEST_VERSION] CONTAINS COMPOSITION c
WHERE v/commit_audit/time_committed/value > $since
ORDER BY v/commit_audit/time_committed/value, v/uid/value
```

- **`$since`** is the instant of the last row the previous run loaded
  (`etl run --since`). The answer holds every composition whose latest version
  was committed after it: a new composition, and a new version of an older
  one.
- **The order** is the commit instant, then the version uid. Every version of
  one contribution carries the same `time_committed` (RM common
  `master06-change_control_package.adoc` §Contributions), so the instant alone
  leaves ties. Under `LATEST_VERSION` each versioned object answers one
  version, so the uid makes the order total, the same order as
  `time_committed`, `vo_id` and version number.
- **Paging** uses the `offset` and `fetch` members of the request. With a total
  order, consecutive windows concatenate to the whole answer, with no row
  skipped or repeated.
- **The plan** runs the `time_committed` predicate as a range scan on a BRIN
  index over the commit instant, so a batch reads the versions committed after
  `$since`, not the whole store.

## What leaves the batch

The batch is a population query: it names no `ehr_id`. So the marks on
[Retention, restriction and objection](retention.md) take an EHR out of it:

- an EHR or object under a [restriction of processing](retention.md#restriction-of-processing)
  (GDPR Art. 18);
- an EHR under a [research objection](retention.md#objection-to-research-processing)
  (GDPR Art. 21(6)), unless the controller recorded the public-interest
  override;
- an EHR whose `EHR_STATUS.is_queryable` is false (RM ehr master04 §EHR
  Status).

The rows carry what the query projects. The query above projects the
`ehr_id`, the uids and the composition, and no identifier of the subject: the
subject's identifiers live in `EHR_STATUS` and in the demographic domain,
which the batch does not read.

The integration test `secondary_use_batch` in `app/ferroehr/tests/it/` holds
this contract. It commits versions before and after an instant, restricts one
EHR and records an objection on another. It then checks that the batch answers
exactly the later versions of the unmarked EHR, in order, that its offset
windows make up the whole answer, and that no row carries the subject's
identifier.

## Removing what was erased

ITS-REST 1.1.0 defines no change notification, so FerroBRIDGE learns of an
erasure by asking. Each run reconciles every composition its watermarks name
against `GET /ehr/{ehr_id}/composition/{versioned_object_uid}`. An answer
that the composition is gone removes its rows from the CDM. A restriction or
an objection set after a row was loaded takes the EHR out of later batches;
removing rows already loaded is FerroBRIDGE's side of the contract.

## The access record

Every batch, export and cohort query is recorded in the
[access log](../audit.md) with the purpose of use the caller declared
(`[audit] purpose_header`) and the data permit it acts under
(`[audit] permit_header`, default `x-data-permit`). A health data access body
issues the permit (EHDS Art. 68). The record keeps the permit's identifier, 1 to
128 characters of letters, digits and `.` `_` `:` `/` `#` `-`. A value of any
other form is recorded as absent, so a header cannot carry free text or
personal data into the trail. The permit joins the tamper chain's digest with
the rest of the record.

A deployment that serves secondary use sets both headers on FerroBRIDGE's
requests, and agrees the purpose codes in `[audit] purpose_codes`.

## Where pseudonymisation happens

FerroBRIDGE pseudonymises for research, as the trusted party between the
holder and the researcher (EDPB Guidelines 01/2025 on pseudonymisation,
§§ 116 to 120 and Example 5). FerroEHR gates the data: the marks above decide
what leaves, the access record says who took it under which permit, and
[cohort queries](../querying-aql.md#cohort-queries-across-the-pseudonymisation-boundary)
answer an aggregate question without releasing the rows.

## What this page does not claim

FerroEHR is not a health data access body and runs no secure processing
environment. Issuing a data permit, checking the request against it, and
releasing data to a researcher are the access body's work (EHDS Chapter IV).
FerroEHR records the permit the caller names; it does not check that the
permit exists or covers the data.
