---
name: its-xml-document-element-inventory
description: The complete global (document) element inventory of both vendored ITS-XML bundles, what each bundle lacks, and which REST resources therefore have no XML document
metadata:
  type: reference
---

# ITS-XML global document elements — the whole inventory

Bundles: `crates/openehr-its/schemas/xml/its-xml-1.0.2-nsv1/` (STABLE, namespace
`…/v1`) and `…/its-xml-2.0.0-nsv2/` (TRIAL, namespace `…/v2`). (A duplicate
`components/` tree was removed 2026-10-14.) Enumerate with an XML parse for `xs:element` children of
`xs:schema` (indent-based grep is unreliable — the files mix tabs and spaces).

**1.0.2 bundle (flat `ALL/` + `AOM2/`) — 8 names:** `composition`, `version`,
`items` (type `LOCATABLE`, the abstract catch-all root), `template`, `extract`,
`extract_request`, `versioned_object`, `archetype`.

**2.0.0 bundle (per-component `RM/Release-*/documents/`, `AM/`, `OET/`,
`QUERY/`) — the same 8 plus `result_set` (type `QUERY_RESPONSE`) and
`query_request`.**

## The hard negatives (these decide several upstream reports)

- **No global element for:** EHR, EHR_STATUS, FOLDER/directory, the five
  demographic PARTY types, PARTY_RELATIONSHIP, CONTRIBUTION, REVISION_HISTORY,
  ITEM_TAG lists. `Resources.md` §"XML Format" nevertheless makes XSD conformance
  a MUST and the `Accept`/`Content-Type` enums offer `application/xml` on them.
- **ITEM_TAG has no complexType at all** in EITHER bundle (`grep -rl ITEM_TAG
  --include='*.xsd'` → nothing).
- **`CONTRIBUTION` complexType exists ONLY in the 2.0.0 lineage**
  (`RM/*/Common.xsd` L183-189: `uid` HIER_OBJECT_ID mandatory, `versions`
  OBJECT_REF maxOccurs=unbounded, `audit` AUDIT_DETAILS) — i.e. a COMMITTED
  contribution, never a commit envelope with inline version data. The 1.0.2
  bundle does not mention CONTRIBUTION anywhere.
- **`UPDATE_VERSION` / `UPDATE_AUDIT` have zero XSD counterpart** in either
  bundle, while ITS-REST `schemas/ehr/NewContribution.yaml` is built from
  `UpdateVersion.yaml` + `common/UpdateAudit.yaml`. So the 1.1.0 commit envelope
  is JSON-only by construction.
- 1.0.2 has **no QUERY schema, no `Ehr.xsd`, no `Demographic.xsd`** (only
  `ALL/{Archetype,BaseTypes,Composition,CompositionTemplate,Content,Extract,
  OpenehrProfile,Resource,Structure,Template,Version}.xsd`).

Cross-refs: [[unknown-key-open-vs-closed-objects-location]] (schema
open-vs-closed), [[contribution-ops-location]] (the JSON commit shape),
[[its-rest-wire-contract-location]] (the 415/406 negotiation MUSTs live in
`ITS-REST/specifications/docs/overview/Resources.md` §"XML Format" L75-83).
