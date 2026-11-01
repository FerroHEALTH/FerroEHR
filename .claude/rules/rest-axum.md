---
paths: ["app/ferroehr-rest/**", "app/ferroehr/src/application/**"]
---

# REST server (axum) — the ITS-REST protocol adapter (shipped; rule governs maintenance/extension)

The server is `axum` 0.8 **implementing the generated ITS-REST contract** from
`openehr-its::rest::generated`. It does not re-declare routes or
DTOs — it provides `impl {Group}Api for AppState` and mounts the generated
router.

## Spec sources (the oracle)

Endpoint semantics (methods, status codes, headers, `Prefer`/`If-Match`
handling, error bodies) come from the vendored spec text — never from memory
or another server: `docs/specs/openehr/ITS-REST/` (the REST spec + API
definitions behind the generated contract) and
the RM, AM, QUERY and TERM texts for the content each endpoint carries (the
SM text under `docs/specs/openehr/SM/` is reference only, never an oracle:
owner ruling 2026-10-27). The CNF chapters
`docs/specs/openehr/CNF/docs/platform_test_schedule/master06`-`master12`
(EHR / COMPOSITION / CONTRIBUTION / DIRECTORY / demographic / querying /
admin) + the Robot suites under `CNF/tests/platform/robot/<API_GROUP>/` are a
STALLED structural GUIDE to which request/response behaviours to cover (openEHR
CNF never released a stable version; the Robot suites are stalled/broken) — NOT
the oracle. The correct wire behaviour comes from the RELEASED ITS-REST docs
text + RM; where the CNF schedule or a Robot data set conflicts with a released
spec, the released spec wins (owner ruling 2026-07-24). Cite the released
sections (spec-adherence.md; `/spec-lookup`).

## Rules

- **Implement the generated server traits.** One `impl` per API group
  (ehr/composition/directory/contribution/query/definition/admin); the handler
  bodies are our service layer. RM payloads
  are `openehr-rm` types; transport DTOs come from `openehr-its`.
- **Middleware = `tower-http`, not hand-rolled:** trace, cors, compression,
  timeout, request-id, sensitive-headers, catch-panic, normalize-path. Auth is a
  `tower`/extractor layer (see `auth.md`).
- **Content negotiation** (canonical JSON + XML) goes through `openehr-its`
  (`to_canonical_json`/`from_canonical_json`, `xml::runtime::to_xml`/
  `from_canonical_xml`); honour `Accept`/`Content-Type`/`Prefer` exactly as
  the ITS-REST spec requires.
- **Errors → responses** via `openehr-its::rest::runtime::ApiError` (it carries
  the ITS-REST status codes). Map service/domain errors into it; return the
  openEHR error body shape the spec defines.
- **OpenAPI:** `ferroehr-rest` serves its own `utoipa`-generated OpenAPI +
  Swagger UI; the vendored OAS is the `emit-rest` codegen input for the
  ITS-REST contract and a subordinate wire source (owner rulings 2026-07-24 +
  2026-07-28: the docs text wins every conflict; the OAS grounds only what
  the docs text is silent on), never a served document (owner ruling,
  2026-07-17).
- **Admin** (`/rest/admin`) lives in `ferroehr-rest`, reusing the same service
  layer. (There is no EhrScape adapter — that surface was cut; the FLAT /
  STRUCTURED / Web-Template simplified formats are served through the standard
  openEHR endpoints via `openehr_sdt::flat`.)

Behaviour is verified by the **CNF pipeline** (the pinned Veredictum instrument — the CNF
schedule text is the oracle it derives from), never by mirroring another
server. Every wire-visible change ends with a `scripts/conformance.sh` run
showing zero drift vs the committed baseline. Build compiling + tested (`rust-style.md`); REST
surface changes are user-visible → same-PR website-book + changelog entries.
