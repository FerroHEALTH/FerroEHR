---
name: draft-classes-apache
description: the generated openEHR draft classes (access control, access log) ship in openehr-rm behind the `draft` feature under Apache-2.0, published to crates.io (owner, 2026-11-14)
metadata:
  type: project
---

The generated draft classes of FerroEHR's access-control and access-log
design (`openehr_rm::draft`) are part of `openehr-rm` behind the `draft`
feature, Apache-2.0 like the rest of that crate, and published to crates.io.
There is no separate draft crate and no licence exception.

**Why:** they are generated from a published specification draft, the same
kind of code as the RM itself.

**How to apply:** a consumer enables `openehr-rm`'s `draft` feature.
Related: [[publish-dispatch-approved]].
