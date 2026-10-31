---
name: authz-class-is-own-design
description: RBAC class assignment is our own design (ITS-REST mandates no scheme), so a 403 is never an app defect — and semantics cases must drive an authorized principal
metadata:
  type: project
---

Which role may call an operation is our own design: no released spec assigns
it, so a role-based 403 can never be an application-bin defect.

ITS-REST `specifications/docs/overview/Requests_and_responses.md`
§Authentication and authorization: "this specification does not mandate a
specific authentication scheme"; its only MUST is the 401/403/407 header
mechanics. The released Admin API `docs/admin/Description.md` carries no
privilege language at all. The Service Model is reference text, never an
authority (owner ruling 2026-10-27), so it grounds nothing here.

**How to apply.** (1) A red row whose only delta is `expected <semantics>,
observed forbidden` is never the application bin — derive the class question as
own-design. (2) A case whose declared outcomes are semantics-only MUST drive a
principal the deployment authorizes: the semantics under test are those of an
authorized call. The `on:` key is ISO/IEC 9646 selection data, not a
spec-derived expectation, so changing it is not the banned "adjust the
expectation to match the SUT". (3) The catalogue's own established pattern is
`schedule/admin/I_ADMIN_SERVICE.physical_ehr_delete-delete_existing.yaml`:
`on: admin` on the destructive step, default `sut` on the verification step.
(4) But a class move ALWAYS costs coverage — the newly-refused principal branch
needs its own case, and any `-readonly_forbidden` row on the moved route stops
isolating the read-only restriction (the Admin gate now refuses first).

See [[definition-delete-routes-are-admin-class]].
