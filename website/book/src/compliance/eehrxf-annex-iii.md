<!-- SPDX-FileCopyrightText: Cadasto B.V. -->
<!-- SPDX-License-Identifier: BUSL-1.1 -->

# EEHRxF metadata from the Reference Model

The draft implementing act on the European electronic health record exchange
format (EHDS Art. 15(1), Have your say initiative 17512, Ares(2026)10884928 of
2026-11-23,
[annexes](https://ec.europa.eu/info/law/better-regulation/api/download/090166e5374c0182))
lists in its Annex III the metadata every exchanged document and every data
group carries. Table 1 is the document metadata, Table 2 the data group
metadata. FerroBRIDGE, the European interoperability software component of
the EHR system, writes them in FHIR R4 from what FerroEHR returns over
ITS-REST. FerroEHR implements no FHIR, so this page records only the openEHR
side: for each element, where the Reference Model holds it, the ITS-REST call
that returns it, and what FerroBRIDGE has to take from somewhere else.

The Reference Model is release 1.2.0
([RM specifications](https://specifications.openehr.org/releases/RM/Release-1.2.0/)):
the EHR and composition packages of the EHR Information Model, the change
control and archetyped packages of the Common Information Model, and the
entry classes. The calls are ITS-REST 1.1.0
([REST API](https://specifications.openehr.org/releases/ITS-REST/Release-1.1.0/)).

<!-- toc -->

> [!WARNING]
> The act is a draft open for feedback until 2026-12-21, and its tables can
> change before adoption. This page follows the draft text. A table that
> names an RM source shows where the data is; it does not show that an
> exchanged document meets the act, which no conformity assessment has
> examined.

## The calls

Each row below names its call by letter.

| Call | Request | Returns |
|---|---|---|
| C | `GET {base}/ehr/{ehr_id}/composition/{uid_based_id}` | the `COMPOSITION` |
| V | `GET {base}/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/version/{version_uid}` | the `ORIGINAL_VERSION`: `uid`, `commit_audit`, `lifecycle_state`, `attestations` and the composition as `data` |
| H | `GET {base}/ehr/{ehr_id}/versioned_composition/{versioned_object_uid}/revision_history` | one `REVISION_HISTORY_ITEM` per version, its commit audit first and each later `ATTESTATION` after it |
| S | `GET {base}/ehr/{ehr_id}/ehr_status` | the `EHR_STATUS` with its `subject` |
| E | `GET {base}/ehr/{ehr_id}` | the `EHR` with its `system_id` |
| P | `GET {base}/demographic/person/{uid_based_id}` | the `PERSON` a `PARTY_REF` names |
| Q | `POST {base}/query/aql` | the same `VERSION` attributes for many compositions in one query ([VERSION paths](../querying-aql.md#version-paths)) |

A composition read also answers the version id in its `ETag`, so C alone
gives the identifier and the version.

## The elements

`State` says how much of the element the Reference Model holds:

- **Sourced:** an RM attribute holds the element and an ITS-REST call
  returns it.
- **Partly sourced:** an RM attribute holds part of it, or holds it only
  where the template or the deployment puts it there.
- **No RM source:** no RM attribute holds it. FerroBRIDGE takes it from
  elsewhere, as the next section says.

| Element | Table | RM source | Call | State |
|---|---|---|---|---|
| `subject` | 1, 2 | `EHR_STATUS.subject`, a `PARTY_SELF` whose `external_ref` names the subject's `PERSON`; on a data group `ENTRY.subject`, which is `PARTY_SELF` when the entry is about the EHR's subject | S, P | Sourced |
| `identifier` | 1, 2 | the `VERSIONED_OBJECT` uid (`HIER_OBJECT_ID`) and the version's `uid` (`OBJECT_VERSION_ID`); on a data group `LOCATABLE.uid` of the entry, where the writer set one | C, V | Sourced |
| `version` | 1, 2 | the `version_tree_id` part of `ORIGINAL_VERSION.uid`, `{object_id}::{creating_system_id}::{version_tree_id}` | C, V | Sourced |
| `author[x]` | 1, 2 | `COMPOSITION.composer`; FerroEHR stores a person's composer as a `PARTY_IDENTIFIED` whose `external_ref` names the `PERSON` ([who is the committer](../security/committer.md)). The organisation: `EVENT_CONTEXT.health_care_facility`, and the healthcare-provider identifier FerroEHR writes on a person's `commit_audit.committer`. The device: `FEEDER_AUDIT.originating_system_audit.system_id`. On a data group `ENTRY.provider` and `ENTRY.other_participations` | C, V, P | Sourced |
| `date` | 1, 2 | `ORIGINAL_VERSION.commit_audit.time_committed` of the version served | V, Q | Sourced |
| `attestation` (`attester[x]`, `dateTime`) | 1, 2 | `ORIGINAL_VERSION.attestations`: each `ATTESTATION`'s `committer` and `time_committed`, with `is_pending` false for a completed one | V, H, Q | Sourced |
| `status` | 1, 2 | `ORIGINAL_VERSION.lifecycle_state` (openEHR `version lifecycle state`: `532` complete, `553` incomplete, `523` deleted) with `commit_audit.change_type` (`250` amendment). A data group's status follows the binding of its own category, which the entry's archetype holds where it has one (for example `ACTION.ism_transition.current_state`) | V, Q | Partly sourced |
| `language` | 1, 2 | `COMPOSITION.language` (ISO 639-1) with `COMPOSITION.territory` (ISO 3166-1), which together form a BCP 47 tag such as `nl-NL`; on a data group `ENTRY.language` | C | Sourced |
| `documentCategory` | 1 | no RM attribute. `COMPOSITION.category` is the openEHR category (`431` persistent, `433` event, `451` episodic), a different axis from the LOINC category | C | No RM source |
| `documentType` | 1 | no RM attribute holds a LOINC document type. The template id, `archetype_details.template_id`, is the key to one | C | No RM source |
| `documentTitle` | 1 | `COMPOSITION.name` | C | Sourced |
| `period` | 1 | `EVENT_CONTEXT.start_time` and `end_time`. A persistent composition has no context, and the element, 0..1, is then absent | C | Sourced |
| `legalAuthentication` (`legalAuthenticator[x]`, `dateTime`) | 1 | no RM class of its own. The nearest is an `ATTESTATION` with `reason` `240` signed and `is_pending` false; which attestation counts as the legal one is the deployment's rule, which the RM does not state | V, H, Q | Partly sourced |
| `eventCategory` | 1 | no RM attribute. The archetype ids of the content entries are the key to one | C | No RM source |
| `serviceSpecialty` | 1 | no RM attribute. `EVENT_CONTEXT.setting` is the openEHR care setting, a different axis from a SNOMED specialty | C | No RM source |
| `custodian` | 1 | no RM attribute. The act asks for the organisation "in charge of maintaining the document"; `EHR.system_id` is "the identifier of the logical EHR management system in which this EHR was created", a system, not an organisation. The healthcare-provider identifier on a person's `commit_audit.committer` names who committed a version (EHDS Art. 13(4)), which for a federated write is the external professional's provider | E | No RM source |
| `clinicalEventTime[x]` | 2 | on the entry: `OBSERVATION.data` (`HISTORY.origin` and each `EVENT.time`), `ACTION.time`. An `EVALUATION`, an `INSTRUCTION` or an `ADMIN_ENTRY` holds a clinical time only in an archetyped element, such as an onset | C | Partly sourced |
| `lastUpdate` | 2 | `commit_audit.time_committed` of the newest version of the composition. The RM versions a composition whole, so a data group's last update is that of its composition | H, Q | Sourced |

Ten of the eighteen elements are sourced, three partly and five have no RM
source. FerroBRIDGE supplies the eight that are not sourced, as the next
section says: from its mapping of the template and the archetypes, from its
configuration, or by the rule of the jurisdiction.

## What FerroBRIDGE takes from elsewhere

- **`documentCategory`, `documentType` and `eventCategory`:** from its
  mapping of the template, keyed by `archetype_details.template_id` for the
  document and by the content archetype ids for the event, with the LOINC,
  SNOMED GPS or DICOM codes the act binds. A template whose
  `COMPOSITION.name` carries a `TERM_MAPPING` to LOINC gives the document
  type in the composition itself.
- **`serviceSpecialty`:** from the deployment's configuration of the care
  units, or from an archetyped element in `EVENT_CONTEXT.other_context` where
  the template places one.
- **`custodian`:** the healthcare provider that deploys the EHR system, from
  FerroBRIDGE's configuration. A FerroEHR instance serves one organisation
  ([one instance per organisation](../security.md#one-instance-one-organisation)), so that provider maintains every document
  the instance holds: the custodian belongs to the deployment, and no
  document or version records it. FerroEHR's `[authz.party_binding.provider]`
  names the same organisation, and no ITS-REST call returns it; reading it off
  a committer would name a federated writer's provider for that writer's
  versions. FerroBRIDGE, deployed for the same data holder, carries it in its
  own configuration.
- **`legalAuthentication`:** the rule of the jurisdiction that says which
  attestation authenticates a document legally. FerroBRIDGE applies it to the
  attestations call V returns.
- **A data group's `status` and `clinicalEventTime`:** from the entry's
  archetype, per data group, as FerroBRIDGE's mapping of that archetype says.

FerroBRIDGE's own record of the Annex III mapping, from the openEHR
composition to the FHIR `Composition` and `DocumentReference`, is
[FerroHEALTH/FerroBRIDGE#2080](https://github.com/FerroHEALTH/FerroBRIDGE/issues/2080).
