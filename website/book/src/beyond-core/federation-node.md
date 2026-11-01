# FerroEHR as a federation node

FerroFED is the FerroHEALTH federation gateway: a client sends one AQL query to
it, and it dispatches the query to the openEHR CDRs it federates. FerroEHR is
one of those CDRs, a **node**. This page is the node contract: what a
FerroEHR node exposes, what it requires, and what it refuses. The gateway's
own behaviour is FerroFED's and is documented there.

The ground is the openEHR **Federation Tier with AQL** specification, release
1.0.0, approved by the openEHR SEC on 2026-10-12
(<https://github.com/syntaric/openehr-federation-spec>). Its requirements are
numbered N1 to N44. Most of them bind the gateway. The rows below are the ones
a node answers.

FerroFED 0.0.13 runs FerroEHR 5.4.1 as its node in its end-to-end harness and
its quickstart.

## Endpoints

A node exposes the openEHR REST API (ITS-REST 1.1.0) under its own base URL
(`[server] base_path`; N28). The gateway uses:

| Use | Endpoint |
|---|---|
| The dispatched query, scoped to the resolved EHR (N7, N33) | `POST {base}/query/aql` with `ehr_id`, or a `WHERE e/ehr_id/value = …` predicate (N29) |
| A follow-up read by version uid (N22) | `GET {base}/ehr/{ehr_id}/composition/{uid}` and the other resource reads |
| A routed write (N23) | the ITS-REST commit operations; `ETag` and `Location` come back for the gateway to pass through (N31) |
| The node's `system_id` and release | `OPTIONS {base}/` and `GET {base}/` (ITS-REST system information) |
| Moving a record between nodes | `GET /ferroehr/rest/extensions/v1/message/export/{ehr_id}` and `POST /ferroehr/rest/extensions/v1/message/import?ehr_id=…` ([EHR Extract & messaging](messaging.md)) |

The node answers an ITS-REST `RESULT_SET` (N17). It runs `WHERE`, `ORDER BY`,
`LIMIT` and `OFFSET` locally; merging pages across nodes is the gateway's work
(N9, N39).

## Identity

**The node is invoked on its local `ehr_id` alone** (N33, N34). It never needs
the patient's identifier in the query: the gateway resolves the patient to
`{node, local ehr_id}` pairs outside AQL, through IHE PIXm or a record locator
(FerroPIX). The node's own cross-reference is the IHE identity source asked
over PIXm, or the `EHR_STATUS` subject of the node's own EHRs
([Demographics](demographics.md)).

**A node does not have to store a directly identifying identifier** in
`EHR_STATUS.subject` (N5). With `[privacy] subject_namespaces` it keeps only a
pseudonym, and the identifier scanner refuses a national identifier where it
is configured ([Privacy & data minimisation](../installation/config-privacy.md)).

**Every version keeps its identity across nodes** (N21, N22, N36). An
`OBJECT_VERSION_ID` is `object_id::creating_system_id::version_tree_id` (RM
common master06 §Version Identification). An EHR exported from one node and
imported on another keeps every version uid and its `creating_system_id` byte
for byte; the receiving node records the import as its own act beside the
copied original (RM common master06 §The Copy Operation). The integration test
`federation_node` in `app/ferroehr/tests/it/` moves an EHR between two nodes
and compares every version uid.

## Admission (N42a)

A federation admits a node on four identifier conditions. For three of them,
the operator configures a FerroEHR node as follows:

- **A federation-wide unique `system_id`.** The default, `ferroehr.local`, is
  the same on every node that leaves it unset. Set `[server] system_id` to a
  name no other member uses before the node commits its first version: the
  `system_id` is stamped into every version uid and never changes
  ([Server settings](../installation/config-server.md#system_id-the-data-authoring-identity)).
- **No adoption of a foreign `ehr_id` on import.** `POST /message/import`
  without `ehr_id` re-uses the source EHR's id (RM common master06 §Copying,
  Case 1). A federation member imports with `?ehr_id=` set to an id it chose.
- **No reuse of an `ehr_id`.** A node never issues an id twice; creating an
  EHR under an id it holds is refused (`409 Conflict`, ITS-REST EHR API `PUT /ehr/{ehr_id}`).
- **`ehr_id` generation.** FerroEHR mints every `ehr_id` as a UUIDv7 (RFC 9562
  §5.7).

## Authentication

There is no central FerroHEALTH authorization service. Each node validates the
bearer tokens of the deployment's own authorization server itself, as a SMART
App Launch resource server ([SMART App Launch](../smart-app-launch.md)), and
records the caller in its access log (N24, N25). The gateway propagates the
client's identity; the node decides on it.

## What the node refuses

**Access decisions are the node's** (N26, N27). Whatever the gateway
pre-filtered, the node applies its own controls before it releases data:

- role and attribute rules, and the per-EHR `EHR_ACCESS` decision;
- the restriction of access by health professionals (EHDS Art. 8);
- a [restriction of processing](../compliance/retention.md#restriction-of-processing)
  (GDPR Art. 18): a restricted object leaves every query result and export,
  and a point read of it answers `403`;
- a [research objection](../compliance/retention.md#objection-to-research-processing)
  (GDPR Art. 21(6)): the EHR leaves every population query and export.

A refusal is the node's answer, which the gateway reports for that node. The
run that sets a restriction at a node and shows a federated query never
returns the restricted object belongs in FerroFED's harness
(FerroHEALTH/FerroFED#1389).

## What this page does not claim

FerroEHR implements no gateway: no patient discovery (IHE XCPD), no
cross-node merge, no registry of nodes. Those are FerroFED's and FerroPIX's.
The node answers `ehr_get_by_subject` only for an identifier its own
cross-reference already holds.
