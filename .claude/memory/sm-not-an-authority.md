---
name: sm-not-an-authority
description: "Owner ruling 2026-10-27: the openEHR Service Model (SM) is not an authority; the oracles are what goes in and comes out, and the service layer is FerroEHR's own design"
metadata:
  node_type: memory
  type: feedback
---

Owner ruling (2026-10-27, issue #4211): SM describes internal service
interfaces, and openEHR holds no implementation to them. The oracles are the
specifications of what goes in and comes out: BASE, RM, AM, TERM, QUERY/AQL,
ITS-REST, ITS-JSON, ITS-XML and the serialisation formats. How FerroEHR is
built inside is its own design.

**How to apply:** never justify, add or keep a feature because SM names it;
never cite SM as the ground for a decision (cite the wire specification, or
write "no specification governs this: our own design"); never mark something
"not realised" against SM as if it were a conformance gap. CNF case cores
keep SM operation names as identifiers only. The vendored SM text under
`docs/specs/openehr/SM/` stays as reference with its provenance. This
supersedes the 2026-07-24 ruling that made SM an oracle beside the ITS-REST
docs text, and retires the SM chapter-audit program in
[[spec-chapter-audit-programs]].
