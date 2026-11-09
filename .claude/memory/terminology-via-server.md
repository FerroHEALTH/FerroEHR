---
name: terminology-via-server
description: Code translation in production always goes through a FHIR terminology server (FerroTERM is the reference) over ferrobridge-term; an embedded or offline value map is for tests only (owner 2026-11-09)
metadata:
  type: feedback
---

On 2026-11-09 the owner overruled a plan to map FerroPIX's PID offline from the v2-to-FHIR ConceptMaps embedded in `ferrobridge-hl7v2`: "why the fuck do we not want a terminology server ... without terminology server makes absolutely no sense". Offline means without a CDR, never without terminology.

**Why:** the family's design puts code systems, value sets and ConceptMaps in the terminology server ([[family-architecture]]); a second copy inside a crate or a product drifts from it.

**How to apply:** split by what a map is. A value map (code A to code B, an HL7 v2 table, a local code to SNOMED CT or LOINC) goes only through `ferrobridge-term` (`$translate`, `$lookup`, `$validate-code`) against the configured terminology server, never an embedded, offline or hand-written copy. A structure map (which field goes where) is mapping logic, generated at build time from its vendored, pinned source, never loaded at runtime from a folder or a server. Tests use terminology stubs or a FerroTERM container. Never propose "no terminology server" as a product posture.
