---
name: spec-lookup
description: >
  Finds and reads the authoritative openEHR spec text for a topic (an RM
  class, attribute, invariant, REST endpoint, AQL construct, data type,
  status-code rule, or CNF test case) in the vendored specs at
  docs/specs/openehr/. Use before implementing or reviewing any spec-facing
  behaviour, when a spec question comes up, or when the user asks "what does
  the spec say about X".
allowed-tools: [Read, Grep, Glob, Bash]
argument-hint: "<class / attribute / endpoint / AQL construct / topic>"
---

# /spec-lookup

Answer spec questions from the vendored normative text at
`docs/specs/openehr/` — never from memory. Every answer cites file + section.

## Procedure

1. **Route to the owning component(s)** (map in `docs/specs/openehr/README.md`):
   - RM classes / invariants / versioning semantics → `RM/docs/`
     (`ehr`, `common`, `data_types`, `data_structures`, `demographic`, `support`)
   - Primitives, ISO 8601, intervals, identifiers (`OBJECT_VERSION_ID`, …) → `BASE/docs/`
   - REST endpoints, status codes, headers, representations → `ITS-REST/`
     (the SM text under `SM/docs/openehr_platform/` is reference only, never
     an oracle: owner ruling 2026-10-27)
   - AQL grammar/semantics/functions/result set → `QUERY/docs/AQL/`
   - Archetypes, templates, OPT 1.4, ADL, AOM constraints → `AM/docs/`
   - FLAT / STRUCTURED / SDT / web template → `SM/docs/serial_data_formats/`,
     `SM/docs/simplified_im_b/`
   - Terminology code sets / term sets → `TERM/docs/SupportTerminology/`
   - BMM / ODIN → `LANG/docs/`
   - Canonical JSON schema shapes → `ITS-JSON/components/`
   - **Conformance expectations** (what a CNF-conformant server must return) →
     `CNF/docs/platform_test_schedule/master*.adoc` + the executable Robot
     suites in `CNF/tests/platform/robot/<API_GROUP>/`
2. **Grep** the exact spec-cased name (e.g. `DV_QUANTITY`, `preceding_version_uid`,
   `LATEST_VERSION`) across the component's `docs/**/*.adoc`; specs use
   openEHR upper-snake class names. Try attribute names and section keywords
   if the class name is too noisy.
3. **Read the surrounding section**, not just the matching line: the class
   definition table, its **invariants**, inherited-class semantics (check the
   ancestor's section too), and any "business rules" prose near it.
4. **Cross-check CNF** whenever the topic is server behaviour: find the test
   case ids covering it and note the exact expected status codes/payloads.
5. **Answer with citations**: `docs/specs/openehr/<component>/docs/<file>.adoc`
   + the section heading (and CNF test-case ids where applicable). If the spec
   is genuinely silent or ambiguous, say so explicitly — that is the signal to
   record a `// NOTE:` decision, with EHRbase consulted only as
   prior art.

## Figures are normative and NOT uniformly text-extractable

Some vendored spec figures carry normative content found nowhere in prose
(the RM version-lifecycle state machine's nine transitions, four of which no
text names). Extraction method varies PER FIGURE — never assume one works:

- Path-rendered SVGs (`RM-version_lifecycle.svg`): zero text nodes — rasterize
  (`rsvg-convert -w 3200`) and READ AS AN IMAGE.
- draw.io exports (`ehr/diagrams/tags_example.svg`): labels live in
  `<foreignObject>` HTML; the `<text>` nodes are truncated stubs ("Viewer does
  not support full SVG 1.1") — grep the foreignObject content, or rasterize.
- Some figures are referenced via the remote `{uml_diagrams_uri}` and are NOT
  vendored at all (ch.7 `RM-common.tags.svg`) — say so honestly rather than
  claiming figure content.

A `<text>`-only extraction that returns nothing is a signal to try the other
methods, never evidence the figure is empty.
