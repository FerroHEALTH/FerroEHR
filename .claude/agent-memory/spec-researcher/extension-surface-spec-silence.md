---
name: extension-surface-spec-silence
description: Where (not) to look for openEHR governance of extra-spec server endpoints — ITS-REST is totally silent on additional/vendor resources; System API docs is a stub; health/status/management/OAS-serving have zero spec anchor anywhere
metadata:
  type: reference
---

Reference text only where it cites the Service Model: SM is not an authority
(owner ruling 2026-10-27); an SM line here grounds no verdict and no feature.

Routing answer for "does any openEHR spec govern a server serving endpoints
beyond the released ITS-REST operations?" — **NO released clause exists**.
Confirmed first-hand 2026-07-27 across the vendored tree.

## Where the (non-)answer lives

- `docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`
  §(preamble) + §Resource identification — defines a resource as "an instance
  object of a specific openEHR class (type)" and enumerates identifier types.
  Nothing constrains the URI space, forbids extra paths, or defines a
  vendor/extension namespace.
- `.../docs/overview/Requests_and_responses.md` §HTTP status codes — the ONLY
  "additional X MAY be used" sentence in the overview ("Additional status codes
  MAY be used as long as they do not conflict with the predefined codes"). It
  is about **status codes, not paths** — do not mis-cite it as permission for
  extension endpoints.
- `.../docs/overview/Resources.md` §Data representation — "other alternative
  formats MAY be supported as well" (formats, not resources).
- Grep for `additional|extension|vendor|custom|proprietary|non-standard` across
  `ITS-REST/specifications/docs/overview/*.md` returns only the above + the
  deprecated-custom-header note. CNF `docs/**` returns ONE unrelated hit
  (master17.4, ISO 8601-2 extensions).

## Component surfaces with ZERO spec anchor (any component)

health probes, an operational status document, an ops-introspection/metrics
surface, an OAS/Swagger-serving endpoint, multi-tenancy, event subscriptions,
FHIR connectors. None of RM/BASE/AM/QUERY/TERM/SM/ITS-REST/ITS-XML/CNF names
any of them as an authority. Terminology is the closest case: SM
`master12-terminology_service.adoc` (`I_TERMINOLOGY_SERVICE`, 9 calls) describes
operation semantics, but SM is reference text only and no released ITS-REST
wire contract exists, so both the semantics and the wire shape are own design.

## System API

`ITS-REST/specifications/docs/system/Description.md` is the WHOLE docs text —
Purpose / Related Documents / Status (`STABLE`) only. No operation prose, no
manifest field list, no statement that the conformance manifest must (or must
not) advertise non-standard endpoint groups. See also
[[its-rest-wire-contract-location]].

Related: [[admin-api-location]] (the only 2 released admin ops — everything
else under `/admin/` is extension), [[smart-app-launch-location]],
[[demographic-api-location]] (PARTY_RELATIONSHIP has no ITS-REST operation).
